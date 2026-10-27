//! The simulated device: named tags, a ramp rule, a seeded RANDOM source, a SINK counter
//! and the settings the simulator connection reads.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::{ErrorKind, Operation, PlcError, PlcValue, PlcValueType, Result, TagId};

use crate::sim::{FailureScript, FailureStep};

mod random;

use self::random::XorShift;

/// Seed of a device that was not given one: fixed, so an unconfigured simulator is still
/// reproducible.
const DEFAULT_SEED: u64 = 0x5EED_5EED_5EED_5EED;

/// The numeric types a ramp can step.
const RAMP_TYPES: [PlcValueType; 10] = [
    PlcValueType::I8,
    PlcValueType::I16,
    PlcValueType::I32,
    PlcValueType::I64,
    PlcValueType::U8,
    PlcValueType::U16,
    PlcValueType::U32,
    PlcValueType::U64,
    PlcValueType::F32,
    PlcValueType::F64,
];

/// `value + step` in the value's own type, or `None` when the ramp cannot advance.
///
/// Integers use `checked_add`; floats advance only while the sum stays finite. `None`
/// also covers a pair of different or non-numeric types, which `SimDevice::ramp` refuses
/// before a ramp exists.
fn advance(value: &PlcValue, step: &PlcValue) -> Option<PlcValue> {
    match (value, step) {
        (PlcValue::I8(v), PlcValue::I8(s)) => v.checked_add(*s).map(PlcValue::I8),
        (PlcValue::I16(v), PlcValue::I16(s)) => v.checked_add(*s).map(PlcValue::I16),
        (PlcValue::I32(v), PlcValue::I32(s)) => v.checked_add(*s).map(PlcValue::I32),
        (PlcValue::I64(v), PlcValue::I64(s)) => v.checked_add(*s).map(PlcValue::I64),
        (PlcValue::U8(v), PlcValue::U8(s)) => v.checked_add(*s).map(PlcValue::U8),
        (PlcValue::U16(v), PlcValue::U16(s)) => v.checked_add(*s).map(PlcValue::U16),
        (PlcValue::U32(v), PlcValue::U32(s)) => v.checked_add(*s).map(PlcValue::U32),
        (PlcValue::U64(v), PlcValue::U64(s)) => v.checked_add(*s).map(PlcValue::U64),
        (PlcValue::F32(v), PlcValue::F32(s)) => {
            Some(v + s).filter(|sum| sum.is_finite()).map(PlcValue::F32)
        }
        (PlcValue::F64(v), PlcValue::F64(s)) => {
            Some(v + s).filter(|sum| sum.is_finite()).map(PlcValue::F64)
        }
        _ => None,
    }
}

/// One simulated value with its access flags, optional ramp and optional read counter.
///
/// `value` is the value the next served read returns. A tag made with [`SimTag::new`] is
/// readable and writable; [`read_only`](Self::read_only) clears `writable`. The fields
/// are public so a test can build the tag it wants; the type is `#[non_exhaustive]`
/// because the ramp and the counter are private state.
///
/// `Debug` prints the value's type, the flags and whether a ramp or counter is attached,
/// never the value itself.
///
/// # Examples
/// ```
/// use stonecrab::PlcValue;
/// use stonecrab::sim::SimTag;
///
/// let mut tag = SimTag::new(PlcValue::I16(7));
/// assert!(tag.readable && tag.writable);
/// assert_eq!(tag.serve_read()?, PlcValue::I16(7));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
/// Stonecrab-owned; the counter is the `std`-only form of ARCHITECTURE.md 5.1's
/// `read_side_effect`.
#[derive(Clone)]
#[non_exhaustive]
pub struct SimTag {
    /// The value the next served read returns (a ramp advances it after each read).
    pub value: PlcValue,
    /// Whether reads are served. A tag that is not readable refuses them.
    pub readable: bool,
    /// Whether writes are accepted. [`SimDevice::write`] enforces it.
    pub writable: bool,
    ramp_step: Option<PlcValue>,
    reads: Option<Arc<AtomicU32>>,
}

impl SimTag {
    /// A readable, writable tag holding `value`, with no ramp and no counter.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::SimTag;
    ///
    /// let tag = SimTag::new(PlcValue::Bool(true));
    /// assert_eq!(tag.value, PlcValue::Bool(true));
    /// ```
    #[must_use]
    pub const fn new(value: PlcValue) -> Self {
        Self {
            value,
            readable: true,
            writable: true,
            ramp_step: None,
            reads: None,
        }
    }

    /// The same tag with `writable` cleared.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::SimTag;
    ///
    /// let tag = SimTag::new(PlcValue::U8(1)).read_only();
    /// assert!(tag.readable && !tag.writable);
    /// ```
    #[must_use]
    pub const fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// Attach a counter incremented on every served read, and return it.
    ///
    /// The counter is shared with every clone of the tag and with the copy stored in a
    /// device, so a test keeps the `Arc` and observes how many reads reached the tag.
    /// Calling this twice returns the same counter. The count wraps at `u32::MAX`.
    ///
    /// # Examples
    /// ```
    /// use std::sync::atomic::Ordering;
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::SimTag;
    ///
    /// let (mut tag, reads) = SimTag::new(PlcValue::U8(1)).with_read_counter();
    /// tag.serve_read()?;
    /// tag.serve_read()?;
    /// assert_eq!(reads.load(Ordering::Relaxed), 2);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn with_read_counter(mut self) -> (Self, Arc<AtomicU32>) {
        let counter = Arc::clone(self.reads.get_or_insert_with(Arc::default));
        (self, counter)
    }

    /// Serve one read: count it, return the current value, then advance a ramp.
    ///
    /// A ramp is post-increment: the first read returns the start value, the next one
    /// start plus one step, and so on. Overflow rule: when `value + step` would overflow
    /// the type (checked arithmetic for integers; a non-finite sum for floats), the ramp
    /// stops and the value stays at the last value reached, forever. It neither wraps nor
    /// saturates at the type's maximum.
    ///
    /// # Errors
    /// Returns [`ErrorKind::Unsupported`] for [`Operation::Read`] when the tag is not
    /// readable. A refused read is not counted and does not advance a ramp.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let device = SimDevice::new();
    /// device.ramp("count", PlcValue::U8(250), PlcValue::U8(4))?;
    /// assert_eq!(device.serve_read("count")?, PlcValue::U8(250));
    /// assert_eq!(device.serve_read("count")?, PlcValue::U8(254));
    /// // 254 + 4 overflows: the ramp holds at 254.
    /// assert_eq!(device.serve_read("count")?, PlcValue::U8(254));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
    /// Stonecrab-owned; no upstream behavior is claimed.
    pub fn serve_read(&mut self) -> Result<PlcValue> {
        if !self.readable {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Read),
                "the tag is not readable",
            ));
        }
        if let Some(reads) = &self.reads {
            reads.fetch_add(1, Ordering::Relaxed);
        }
        let served = self.value.clone();
        if let Some(next) = self
            .ramp_step
            .as_ref()
            .and_then(|step| advance(&served, step))
        {
            self.value = next;
        }
        Ok(served)
    }
}

impl fmt::Debug for SimTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimTag")
            .field("value_type", &self.value.value_type())
            .field("readable", &self.readable)
            .field("writable", &self.writable)
            .field("ramping", &self.ramp_step.is_some())
            .field("counted", &self.reads.is_some())
            .finish()
    }
}

#[derive(Debug)]
struct DeviceState {
    tags: BTreeMap<String, SimTag>,
    rng: XorShift,
    sink_writes: u64,
    script: FailureScript,
}

/// A simulated device: named [`SimTag`]s, a seeded `RANDOM` source and the settings the
/// simulator connection reads.
///
/// The device is shared (`Arc<SimDevice>`) by the simulator connections, so every method takes
/// `&self` and the tag map sits behind a `std::sync::Mutex`. A poisoned lock is
/// recovered, not propagated: every operation leaves the map consistent before it can
/// panic, so the data is still valid. At most [`SimDevice::MAX_TAGS`] tags are held.
/// `Debug` prints counts, never values.
///
/// The builder methods [`with_latency`](Self::with_latency),
/// [`with_max_items`](Self::with_max_items) and [`with_seed`](Self::with_seed) are
/// stored settings that `SimConnection` reads: the per-operation delay, the
/// `Capabilities` report and the `RANDOM` seed (which `sim://<device>?seed=` replaces
/// with [`reseed`](Self::reseed) when a connection opens).
///
/// # Examples
/// ```
/// use stonecrab::PlcValue;
/// use stonecrab::sim::{SimDevice, SimTag};
///
/// let device = SimDevice::new().with_seed(7);
/// device.set("setpoint", SimTag::new(PlcValue::F32(21.5)))?;
/// assert_eq!(device.serve_read("setpoint")?, PlcValue::F32(21.5));
/// assert_eq!(device.tags().len(), 1);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
/// Stonecrab-owned device model; the seeded `RANDOM` source is `SIM-RANDOM-SEEDED`.
pub struct SimDevice {
    state: Mutex<DeviceState>,
    latency: Duration,
    max_items: Option<NonZeroUsize>,
    seed: u64,
}

impl SimDevice {
    /// Most tags a device holds; `set` and `ramp` of a new name past it fail.
    pub const MAX_TAGS: usize = 4096;

    /// An empty device with no latency, no item limit and the default seed.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::SimDevice;
    ///
    /// let device = SimDevice::new();
    /// assert!(device.tags().is_empty());
    /// assert_eq!(device.latency().as_millis(), 0);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(DeviceState {
                tags: BTreeMap::new(),
                rng: XorShift::new(DEFAULT_SEED),
                sink_writes: 0,
                script: FailureScript::new(),
            }),
            latency: Duration::ZERO,
            max_items: None,
            seed: DEFAULT_SEED,
        }
    }

    /// Seed the `RANDOM` source. Two devices with the same seed serve the same values.
    /// A zero seed is replaced by a fixed nonzero constant.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValueType;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let (a, b) = (SimDevice::new().with_seed(9), SimDevice::new().with_seed(9));
    /// assert_eq!(a.random(PlcValueType::U32)?, b.random(PlcValueType::U32)?);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-RANDOM-SEEDED
    /// Upstream draws from `SecureRandom`, which cannot be seeded; seeding is a divergence.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self.lock().rng = XorShift::new(seed);
        self
    }

    /// Store the per-operation latency the connection applies through its clock.
    #[must_use]
    pub const fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Store the item limit the connection reports as `Capabilities.max_items_per_request`
    /// and enforces on every request.
    #[must_use]
    pub const fn with_max_items(mut self, max_items: NonZeroUsize) -> Self {
        self.max_items = Some(max_items);
        self
    }

    /// The stored latency; zero unless set.
    #[must_use]
    pub const fn latency(&self) -> Duration {
        self.latency
    }

    /// The stored item limit, if any.
    #[must_use]
    pub const fn max_items(&self) -> Option<NonZeroUsize> {
        self.max_items
    }

    /// The seed the `RANDOM` source started from.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Store `tag` under `name`, replacing any tag already there.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `name` is not a valid [`TagId`], and
    /// [`ErrorKind::ResourceLimit`] when a new name would exceed [`MAX_TAGS`](Self::MAX_TAGS).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcValue};
    /// use stonecrab::sim::{SimDevice, SimTag};
    ///
    /// let device = SimDevice::new();
    /// device.set("a", SimTag::new(PlcValue::U8(1)))?;
    /// let refused = device.set(" a", SimTag::new(PlcValue::U8(1)));
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
    /// Stonecrab-owned; no upstream behavior is claimed.
    pub fn set(&self, name: &str, tag: SimTag) -> Result<()> {
        TagId::new(name)?;
        let mut state = self.lock();
        check_capacity(&state, name)?;
        state.tags.insert(name.to_owned(), tag);
        drop(state);
        tracing::debug!(name, "sim tag set");
        Ok(())
    }

    /// Remove and return the tag stored under `name`, if there is one.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::{SimDevice, SimTag};
    ///
    /// let device = SimDevice::new();
    /// device.set("a", SimTag::new(PlcValue::U8(1)))?;
    /// assert!(device.remove("a").is_some());
    /// assert!(device.remove("a").is_none());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn remove(&self, name: &str) -> Option<SimTag> {
        let removed = self.lock().tags.remove(name);
        tracing::debug!(name, found = removed.is_some(), "sim tag removed");
        removed
    }

    /// Make `name` a ramp: its value is `start` and each served read then advances it by
    /// `step`, in the tag's own type (see [`SimTag::serve_read`] for the overflow rule).
    ///
    /// An existing tag keeps its flags and read counter and takes the new value and ramp;
    /// otherwise a readable, writable tag is created. `start` and `step` must have the
    /// same numeric type (`i8`..`i64`, `u8`..`u64`, `f32`, `f64`); an unsigned type can
    /// only count up.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `name` is not a valid [`TagId`] or when
    /// `start` and `step` differ in type or are not numeric, and
    /// [`ErrorKind::ResourceLimit`] when a new name would exceed [`MAX_TAGS`](Self::MAX_TAGS).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let device = SimDevice::new();
    /// device.ramp("level", PlcValue::I32(10), PlcValue::I32(-2))?;
    /// assert_eq!(device.serve_read("level")?, PlcValue::I32(10));
    /// assert_eq!(device.serve_read("level")?, PlcValue::I32(8));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
    /// Stonecrab-owned; ARCHITECTURE.md 5.1 sketches `ramp(name, from, to, period)`, which
    /// needs a clock; the alpha ramp is per read and clock-free.
    pub fn ramp(&self, name: &str, start: PlcValue, step: PlcValue) -> Result<()> {
        TagId::new(name)?;
        let ramp_type = start.value_type();
        if ramp_type != step.value_type() || !RAMP_TYPES.contains(&ramp_type) {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "a ramp needs a start and a step of the same numeric type",
            ));
        }
        let mut state = self.lock();
        check_capacity(&state, name)?;
        let tag = state
            .tags
            .entry(name.to_owned())
            .or_insert_with(|| SimTag::new(start.clone()));
        tag.value = start;
        tag.ramp_step = Some(step);
        drop(state);
        tracing::debug!(name, "sim ramp set");
        Ok(())
    }

    /// A copy of the tag stored under `name`. The copy shares a read counter with the
    /// stored tag but not its value: reading the copy does not advance the stored ramp.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<SimTag> {
        self.lock().tags.get(name).cloned()
    }

    /// Copies of every tag, ordered by name.
    #[must_use]
    pub fn tags(&self) -> Vec<(String, SimTag)> {
        self.lock()
            .tags
            .iter()
            .map(|(name, tag)| (name.clone(), tag.clone()))
            .collect()
    }

    /// Serve a read of the stored tag `name` (see [`SimTag::serve_read`]).
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] when no tag has that name (a STATE value
    /// that was never set) and [`ErrorKind::Unsupported`] for [`Operation::Read`] when
    /// the tag is not readable.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let unset = SimDevice::new().serve_read("never set");
    /// assert_eq!(unset.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
    /// Stonecrab-owned; no upstream behavior is claimed.
    pub fn serve_read(&self, name: &str) -> Result<PlcValue> {
        match self.lock().tags.get_mut(name) {
            Some(tag) => tag.serve_read(),
            None => Err(PlcError::new(
                ErrorKind::InvalidAddress,
                "no tag with that name has been set",
            )),
        }
    }

    /// Replace `value` of the stored tag `name`, the way a write to a `STATE` address does.
    ///
    /// The new value must have the stored value's type exactly (no widening). A ramp keeps
    /// stepping from the new value. The check and the replacement happen under one lock, so
    /// a concurrent reader sees either the old value or the new one.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] when no tag has that name,
    /// [`ErrorKind::Unsupported`] for [`Operation::Write`] when the tag is not writable, and
    /// [`ErrorKind::InvalidConfig`] when `value` has another type than the stored value. A
    /// refused write changes nothing.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcValue};
    /// use stonecrab::sim::{SimDevice, SimTag};
    ///
    /// let device = SimDevice::new();
    /// device.set("setpoint", SimTag::new(PlcValue::I16(20)))?;
    /// device.write("setpoint", PlcValue::I16(25))?;
    /// assert_eq!(device.serve_read("setpoint")?, PlcValue::I16(25));
    /// let wrong = device.write("setpoint", PlcValue::U16(1));
    /// assert_eq!(wrong.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-DEVICE-MODEL
    /// Stonecrab-owned; no upstream behavior is claimed.
    pub fn write(&self, name: &str, value: PlcValue) -> Result<()> {
        match self.lock().tags.get_mut(name) {
            None => Err(PlcError::new(
                ErrorKind::InvalidAddress,
                "no tag with that name has been set",
            )),
            Some(tag) if !tag.writable => Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                "the tag is not writable",
            )),
            Some(tag) if tag.value.value_type() != value.value_type() => Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "the value's type differs from the stored tag's type",
            )),
            Some(tag) => {
                tag.value = value;
                Ok(())
            }
        }
    }

    /// Count one write to a `SINK` address. The value is discarded; only the count stays.
    ///
    /// `SimConnection` calls this for every confirmed `SINK` write. The count saturates at
    /// `u64::MAX`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::SimDevice;
    ///
    /// let device = SimDevice::new();
    /// device.record_sink_write();
    /// assert_eq!(device.sink_writes(), 1);
    /// ```
    pub fn record_sink_write(&self) {
        let mut state = self.lock();
        state.sink_writes = state.sink_writes.saturating_add(1);
    }

    /// How many writes to `SINK` addresses have been counted and discarded.
    #[must_use]
    pub fn sink_writes(&self) -> u64 {
        self.lock().sink_writes
    }

    /// Restart the `RANDOM` source from `seed`, as [`with_seed`](Self::with_seed) does at
    /// construction.
    ///
    /// A zero seed is replaced by the same fixed nonzero constant. The generator belongs to
    /// the device, not to a connection, so every connection to the device shares it;
    /// opening `sim://<device>?seed=<n>` reseeds it. [`seed`](Self::seed) keeps reporting
    /// the seed given at construction.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValueType;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let (a, b) = (SimDevice::new().with_seed(3), SimDevice::new());
    /// b.reseed(3);
    /// assert_eq!(a.random(PlcValueType::U64)?, b.random(PlcValueType::U64)?);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-RANDOM-SEEDED
    /// Upstream draws from `SecureRandom`, which cannot be seeded; seeding is a divergence.
    pub fn reseed(&self, seed: u64) {
        self.lock().rng = XorShift::new(seed);
    }

    /// The next `RANDOM` value of `value_type`, from the device's seeded generator.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for a type outside the eleven scalar types
    /// of the address grammar; the generator still advances.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValueType;
    /// use stonecrab::sim::SimDevice;
    ///
    /// let device = SimDevice::new();
    /// assert_eq!(device.random(PlcValueType::F64)?.value_type(), PlcValueType::F64);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-RANDOM-SEEDED
    /// Upstream draws from `SecureRandom`; the seeded xorshift source is a divergence.
    pub fn random(&self, value_type: PlcValueType) -> Result<PlcValue> {
        self.lock().rng.value(value_type)
    }

    /// Queue a scripted fault for a later connect, read or write.
    ///
    /// The script belongs to the device, so every connection to it draws from the same
    /// queue, in the order operations reach the device. Each operation takes at most one
    /// step; see [`FailureStep`] for what each one does. Push steps after the connection
    /// is open: a `connect` also takes a step (only `Delay`, `Timeout` and `Disconnect` mean
    /// anything to it; the others are consumed and ignored).
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] when [`FailureScript::MAX_STEPS`] steps are
    /// already queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureStep, SimDevice};
    ///
    /// let device = SimDevice::new();
    /// device.push_failure(FailureStep::Timeout)?;
    /// assert_eq!(device.pending_failures(), 1);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-FAILURE-SCRIPT
    /// Stonecrab-owned; the upstream simulated driver has no failure injection.
    pub fn push_failure(&self, step: FailureStep) -> Result<()> {
        self.lock().script.push(step)
    }

    /// Queue several scripted faults in order, or none of them.
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] when the steps would not fit under
    /// [`FailureScript::MAX_STEPS`]; nothing is queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureStep, SimDevice};
    ///
    /// let device = SimDevice::new();
    /// device.push_failures([FailureStep::Timeout, FailureStep::Timeout])?;
    /// assert_eq!(device.pending_failures(), 2);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-FAILURE-SCRIPT
    /// Stonecrab-owned; the upstream simulated driver has no failure injection.
    pub fn push_failures(&self, steps: impl IntoIterator<Item = FailureStep>) -> Result<()> {
        self.lock().script.push_all(steps)
    }

    /// How many scripted faults are still queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::SimDevice;
    ///
    /// assert_eq!(SimDevice::new().pending_failures(), 0);
    /// ```
    #[must_use]
    pub fn pending_failures(&self) -> usize {
        self.lock().script.len()
    }

    /// Drop every queued scripted fault.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureStep, SimDevice};
    ///
    /// let device = SimDevice::new();
    /// device.push_failure(FailureStep::Malformed)?;
    /// device.clear_failures();
    /// assert_eq!(device.pending_failures(), 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn clear_failures(&self) {
        self.lock().script.clear();
    }

    /// The oldest queued fault, removed; what a connection takes for one operation.
    pub(crate) fn take_failure(&self) -> Option<FailureStep> {
        self.lock().script.pop()
    }

    fn lock(&self) -> MutexGuard<'_, DeviceState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for SimDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SimDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimDevice")
            .field("tags", &self.lock().tags.len())
            .field("latency", &self.latency)
            .field("max_items", &self.max_items)
            .finish_non_exhaustive()
    }
}

/// A name that is not stored yet needs room.
fn check_capacity(state: &DeviceState, name: &str) -> Result<()> {
    if state.tags.len() >= SimDevice::MAX_TAGS && !state.tags.contains_key(name) {
        return Err(PlcError::new(
            ErrorKind::ResourceLimit,
            "the device holds the maximum number of tags",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    fn kind<T>(result: Result<T>) -> Option<ErrorKind> {
        result.err().map(|error| error.kind())
    }

    // covers: SIM-DEVICE-MODEL
    #[test]
    fn set_get_remove_and_tags_round_trip() {
        let device = SimDevice::new();
        assert!(device.get("a").is_none(), "nothing before set");
        assert!(
            device.set("b", SimTag::new(PlcValue::U8(2))).is_ok(),
            "set b"
        );
        assert!(
            device.set("a", SimTag::new(PlcValue::U8(1))).is_ok(),
            "set a"
        );
        assert_eq!(
            device.get("a").map(|tag| tag.value),
            Some(PlcValue::U8(1)),
            "get returns the stored value"
        );
        let names: Vec<String> = device.tags().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["a", "b"], "tags are ordered by name");
        assert!(
            device.set("a", SimTag::new(PlcValue::U8(9))).is_ok(),
            "replace a"
        );
        assert_eq!(
            device.serve_read("a").ok(),
            Some(PlcValue::U8(9)),
            "replaced"
        );
        assert_eq!(
            device.remove("a").map(|tag| tag.value),
            Some(PlcValue::U8(9)),
            "remove returns the tag"
        );
        assert!(
            device.remove("a").is_none(),
            "a second remove finds nothing"
        );
        assert!(device.get("a").is_none(), "gone after remove");
        assert_eq!(device.tags().len(), 1, "b remains");
    }

    // covers: SIM-DEVICE-MODEL
    #[test]
    fn names_follow_the_tag_id_rules() {
        let device = SimDevice::new();
        for name in ["", " a", "a ", "a\nb"] {
            assert_eq!(
                kind(device.set(name, SimTag::new(PlcValue::Null))),
                Some(ErrorKind::InvalidConfig),
                "set {name:?}"
            );
            assert_eq!(
                kind(device.ramp(name, PlcValue::U8(0), PlcValue::U8(1))),
                Some(ErrorKind::InvalidConfig),
                "ramp {name:?}"
            );
        }
        let long = "n".repeat(TagId::MAX_BYTES + 1);
        assert_eq!(
            kind(device.set(&long, SimTag::new(PlcValue::Null))),
            Some(ErrorKind::InvalidConfig),
            "over the bound"
        );
        assert!(device.tags().is_empty(), "nothing was stored");
    }

    // covers: SIM-DEVICE-MODEL (resource bound)
    #[test]
    fn the_tag_count_is_bounded_but_replacing_is_always_allowed() {
        let device = SimDevice::new();
        for index in 0..SimDevice::MAX_TAGS {
            assert!(
                device
                    .set(&format!("t{index}"), SimTag::new(PlcValue::Null))
                    .is_ok(),
                "tag {index} fits"
            );
        }
        assert_eq!(
            kind(device.set("one too many", SimTag::new(PlcValue::Null))),
            Some(ErrorKind::ResourceLimit),
            "a new name past the bound is refused"
        );
        assert_eq!(
            kind(device.ramp("one too many", PlcValue::U8(0), PlcValue::U8(1))),
            Some(ErrorKind::ResourceLimit),
            "a new ramp past the bound is refused"
        );
        assert!(
            device.set("t0", SimTag::new(PlcValue::Bool(true))).is_ok(),
            "replace set"
        );
        assert!(
            device.ramp("t1", PlcValue::U8(0), PlcValue::U8(1)).is_ok(),
            "replace as ramp"
        );
        assert_eq!(
            device.tags().len(),
            SimDevice::MAX_TAGS,
            "still at the bound"
        );
        assert!(device.remove("t5").is_some(), "make room");
        assert!(
            device
                .set("one too many", SimTag::new(PlcValue::Null))
                .is_ok(),
            "now it fits"
        );
    }

    // covers: SIM-DEVICE-MODEL (ramp rule)
    #[test]
    fn a_ramp_serves_start_then_adds_the_step_on_every_read() {
        let device = SimDevice::new();
        assert!(
            device
                .ramp("up", PlcValue::I32(10), PlcValue::I32(5))
                .is_ok(),
            "ramp"
        );
        let served: Vec<_> = (0..4)
            .filter_map(|_| device.serve_read("up").ok())
            .collect();
        assert_eq!(
            served,
            [
                PlcValue::I32(10),
                PlcValue::I32(15),
                PlcValue::I32(20),
                PlcValue::I32(25)
            ],
            "post-increment: the first read is the start"
        );
        assert!(
            device
                .ramp("down", PlcValue::I8(0), PlcValue::I8(-3))
                .is_ok(),
            "negative step"
        );
        assert_eq!(
            device.serve_read("down").ok(),
            Some(PlcValue::I8(0)),
            "start"
        );
        assert_eq!(
            device.serve_read("down").ok(),
            Some(PlcValue::I8(-3)),
            "descends"
        );
        assert!(
            device
                .ramp("f", PlcValue::F64(0.5), PlcValue::F64(0.25))
                .is_ok(),
            "float ramp"
        );
        assert_eq!(
            device.serve_read("f").ok(),
            Some(PlcValue::F64(0.5)),
            "float start"
        );
        assert_eq!(
            device.serve_read("f").ok(),
            Some(PlcValue::F64(0.75)),
            "float step"
        );
    }

    // covers: SIM-DEVICE-MODEL (ramp overflow rule: hold at the last value)
    #[test]
    fn a_ramp_that_would_overflow_holds_at_the_last_value() {
        let device = SimDevice::new();
        assert!(
            device.ramp("u", PlcValue::U8(250), PlcValue::U8(4)).is_ok(),
            "u8 ramp"
        );
        let served: Vec<_> = (0..5).filter_map(|_| device.serve_read("u").ok()).collect();
        assert_eq!(
            served,
            [
                PlcValue::U8(250),
                PlcValue::U8(254),
                PlcValue::U8(254),
                PlcValue::U8(254),
                PlcValue::U8(254)
            ],
            "254 + 4 overflows u8: the ramp stops at 254, it neither wraps nor reaches 255"
        );
        assert!(
            device
                .ramp("s", PlcValue::I16(i16::MIN), PlcValue::I16(-1))
                .is_ok(),
            "downward overflow"
        );
        assert_eq!(
            device.serve_read("s").ok(),
            Some(PlcValue::I16(i16::MIN)),
            "start"
        );
        assert_eq!(
            device.serve_read("s").ok(),
            Some(PlcValue::I16(i16::MIN)),
            "holds at the minimum"
        );
        assert!(
            device
                .ramp("big", PlcValue::F32(f32::MAX), PlcValue::F32(f32::MAX))
                .is_ok(),
            "float ramp towards infinity"
        );
        assert_eq!(
            device.serve_read("big").ok(),
            Some(PlcValue::F32(f32::MAX)),
            "start"
        );
        assert_eq!(
            device.serve_read("big").ok(),
            Some(PlcValue::F32(f32::MAX)),
            "a non-finite sum holds the last finite value"
        );
    }

    #[test]
    fn every_numeric_type_ramps_and_holds_at_its_limit() {
        let cases = [
            (PlcValue::I8(i8::MAX), PlcValue::I8(1)),
            (PlcValue::I16(i16::MAX), PlcValue::I16(1)),
            (PlcValue::I32(i32::MAX), PlcValue::I32(1)),
            (PlcValue::I64(i64::MAX), PlcValue::I64(1)),
            (PlcValue::U8(u8::MAX), PlcValue::U8(1)),
            (PlcValue::U16(u16::MAX), PlcValue::U16(1)),
            (PlcValue::U32(u32::MAX), PlcValue::U32(1)),
            (PlcValue::U64(u64::MAX), PlcValue::U64(1)),
            (PlcValue::F32(f32::MAX), PlcValue::F32(f32::MAX)),
            (PlcValue::F64(f64::MAX), PlcValue::F64(f64::MAX)),
        ];
        for (start, step) in cases {
            assert!(
                advance(&start, &step).is_none(),
                "{start:?}: overflow cannot advance"
            );
            let device = SimDevice::new();
            assert!(
                device.ramp("r", start.clone(), step).is_ok(),
                "{start:?}: ramp"
            );
            assert_eq!(
                device.serve_read("r").ok(),
                Some(start.clone()),
                "{start:?}: start"
            );
            assert_eq!(
                device.serve_read("r").ok(),
                Some(start),
                "held at the limit"
            );
        }
        let one = |value: PlcValue| advance(&value, &value).map(|next| next.value_type());
        assert_eq!(one(PlcValue::I8(1)), Some(PlcValueType::I8), "i8 advances");
        assert_eq!(
            one(PlcValue::I16(1)),
            Some(PlcValueType::I16),
            "i16 advances"
        );
        assert_eq!(
            one(PlcValue::I32(1)),
            Some(PlcValueType::I32),
            "i32 advances"
        );
        assert_eq!(
            one(PlcValue::I64(1)),
            Some(PlcValueType::I64),
            "i64 advances"
        );
        assert_eq!(one(PlcValue::U8(1)), Some(PlcValueType::U8), "u8 advances");
        assert_eq!(
            one(PlcValue::U16(1)),
            Some(PlcValueType::U16),
            "u16 advances"
        );
        assert_eq!(
            one(PlcValue::U32(1)),
            Some(PlcValueType::U32),
            "u32 advances"
        );
        assert_eq!(
            one(PlcValue::U64(1)),
            Some(PlcValueType::U64),
            "u64 advances"
        );
        assert_eq!(
            one(PlcValue::F32(1.0)),
            Some(PlcValueType::F32),
            "f32 advances"
        );
        assert_eq!(
            one(PlcValue::F64(1.0)),
            Some(PlcValueType::F64),
            "f64 advances"
        );
    }

    #[test]
    fn advance_refuses_mismatched_and_non_numeric_pairs() {
        assert!(
            advance(&PlcValue::I8(1), &PlcValue::I16(1)).is_none(),
            "different widths"
        );
        assert!(
            advance(&PlcValue::Bool(true), &PlcValue::Bool(true)).is_none(),
            "Boolean"
        );
        assert!(advance(&PlcValue::Null, &PlcValue::Null).is_none(), "null");
        assert!(
            advance(
                &PlcValue::String("a".to_owned()),
                &PlcValue::String("b".to_owned())
            )
            .is_none(),
            "text"
        );
    }

    // covers: SIM-DEVICE-MODEL
    #[test]
    fn ramp_refuses_mismatched_and_non_numeric_inputs() {
        let device = SimDevice::new();
        for (start, step) in [
            (PlcValue::I8(0), PlcValue::I16(1)),
            (PlcValue::F32(0.0), PlcValue::F64(1.0)),
            (PlcValue::Bool(false), PlcValue::Bool(true)),
            (PlcValue::Null, PlcValue::Null),
            (PlcValue::Char('a'), PlcValue::Char('b')),
            (
                PlcValue::String(String::new()),
                PlcValue::String(String::new()),
            ),
            (
                PlcValue::Duration(Duration::ZERO),
                PlcValue::Duration(Duration::from_secs(1)),
            ),
        ] {
            assert_eq!(
                kind(device.ramp("r", start.clone(), step)),
                Some(ErrorKind::InvalidConfig),
                "{start:?}"
            );
        }
        assert!(device.tags().is_empty(), "a refused ramp stores nothing");
    }

    #[test]
    fn ramping_an_existing_tag_keeps_its_flags_and_counter() {
        let device = SimDevice::new();
        let (tag, reads) = SimTag::new(PlcValue::U8(0)).read_only().with_read_counter();
        assert!(device.set("t", tag).is_ok(), "set");
        assert!(
            device.ramp("t", PlcValue::U8(5), PlcValue::U8(1)).is_ok(),
            "ramp over it"
        );
        let stored = device.get("t");
        assert_eq!(
            stored.as_ref().map(|t| t.writable),
            Some(false),
            "read-only kept"
        );
        assert_eq!(
            device.serve_read("t").ok(),
            Some(PlcValue::U8(5)),
            "new start"
        );
        assert_eq!(
            reads.load(Ordering::Relaxed),
            1,
            "the original counter still counts"
        );
    }

    // covers: SIM-DEVICE-MODEL (read counter)
    #[test]
    fn the_read_counter_counts_every_served_read_and_only_those() {
        let device = SimDevice::new();
        let (tag, reads) = SimTag::new(PlcValue::U8(1)).with_read_counter();
        assert!(device.set("t", tag).is_ok(), "set");
        assert_eq!(reads.load(Ordering::Relaxed), 0, "no reads yet");
        for expected in 1..=3_u32 {
            assert!(device.serve_read("t").is_ok(), "read");
            assert_eq!(
                reads.load(Ordering::Relaxed),
                expected,
                "one more read counted"
            );
        }
        let copy = device.get("t");
        assert!(copy.is_some(), "get");
        assert_eq!(
            reads.load(Ordering::Relaxed),
            3,
            "get and tags do not count as reads"
        );
        assert_eq!(device.tags().len(), 1, "tags");
        assert_eq!(
            reads.load(Ordering::Relaxed),
            3,
            "listing does not count either"
        );
        let (same, again) = copy.map(SimTag::with_read_counter).unzip();
        assert!(same.is_some(), "the tag comes back");
        assert!(
            again.is_some_and(|counter| Arc::ptr_eq(&counter, &reads)),
            "asking again returns the same counter"
        );
    }

    // covers: SIM-DEVICE-MODEL
    #[test]
    fn a_tag_that_is_not_readable_refuses_reads_without_counting_or_advancing() {
        let (mut tag, reads) = SimTag::new(PlcValue::U8(1)).with_read_counter();
        tag.readable = false;
        tag.ramp_step = Some(PlcValue::U8(1));
        let error = tag.serve_read().err().map(|e| e.kind());
        assert_eq!(
            error,
            Some(ErrorKind::Unsupported(Operation::Read)),
            "refused"
        );
        assert_eq!(reads.load(Ordering::Relaxed), 0, "not counted");
        assert_eq!(tag.value, PlcValue::U8(1), "ramp not advanced");
        tag.readable = true;
        assert_eq!(
            tag.serve_read().ok(),
            Some(PlcValue::U8(1)),
            "readable again"
        );
    }

    // covers: SIM-DEVICE-MODEL
    #[test]
    fn reading_an_unset_name_is_an_invalid_address() {
        let device = SimDevice::new();
        assert_eq!(
            kind(device.serve_read("missing")),
            Some(ErrorKind::InvalidAddress),
            "unset"
        );
        assert!(device.set("m", SimTag::new(PlcValue::Null)).is_ok(), "set");
        assert!(device.remove("m").is_some(), "remove");
        assert_eq!(
            kind(device.serve_read("m")),
            Some(ErrorKind::InvalidAddress),
            "removed"
        );
    }

    #[test]
    fn a_copy_from_get_does_not_advance_the_stored_ramp() {
        let device = SimDevice::new();
        assert!(
            device.ramp("r", PlcValue::U8(0), PlcValue::U8(1)).is_ok(),
            "ramp"
        );
        let mut copy = device.get("r").expect("the ramp was stored");
        assert_eq!(copy.serve_read().ok(), Some(PlcValue::U8(0)), "copy start");
        assert_eq!(
            copy.serve_read().ok(),
            Some(PlcValue::U8(1)),
            "copy advances"
        );
        assert_eq!(
            device.serve_read("r").ok(),
            Some(PlcValue::U8(0)),
            "stored still at start"
        );
    }

    // covers: SIM-DEVICE-MODEL (stored settings)
    #[test]
    fn latency_max_items_and_seed_are_stored() {
        let default = SimDevice::default();
        assert_eq!(default.latency(), Duration::ZERO, "default latency");
        assert_eq!(default.max_items(), None, "default item limit");
        assert_eq!(default.seed(), DEFAULT_SEED, "default seed");
        let limit = NonZeroUsize::new(8);
        let device = SimDevice::new()
            .with_latency(Duration::from_millis(30))
            .with_max_items(limit.expect("8 is nonzero"))
            .with_seed(99);
        assert_eq!(device.latency(), Duration::from_millis(30), "latency");
        assert_eq!(device.max_items(), limit, "item limit");
        assert_eq!(device.seed(), 99, "seed");
    }

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn random_values_repeat_for_a_fixed_seed_and_differ_between_seeds() {
        let draw = |seed| {
            let device = SimDevice::new().with_seed(seed);
            (0..8)
                .filter_map(|_| device.random(PlcValueType::U64).ok())
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(1234), draw(1234), "same seed, same sequence");
        assert_ne!(
            draw(1234),
            draw(1235),
            "different seeds, different sequences"
        );
        assert_eq!(draw(1234).len(), 8, "every draw succeeded");
        let unseeded = SimDevice::new();
        let fixed = SimDevice::new().with_seed(DEFAULT_SEED);
        assert_eq!(
            unseeded.random(PlcValueType::U64).ok(),
            fixed.random(PlcValueType::U64).ok(),
            "the default seed is fixed"
        );
    }

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn random_serves_every_scalar_type_and_refuses_the_rest() {
        let device = SimDevice::new().with_seed(5);
        for value_type in [
            PlcValueType::Bool,
            PlcValueType::I8,
            PlcValueType::I16,
            PlcValueType::I32,
            PlcValueType::I64,
            PlcValueType::U8,
            PlcValueType::U16,
            PlcValueType::U32,
            PlcValueType::U64,
            PlcValueType::F32,
            PlcValueType::F64,
        ] {
            assert_eq!(
                device.random(value_type).ok().map(|v| v.value_type()),
                Some(value_type),
                "{value_type:?}"
            );
        }
        assert_eq!(
            kind(device.random(PlcValueType::String)),
            Some(ErrorKind::InvalidConfig),
            "text is refused"
        );
    }

    #[test]
    fn debug_prints_counts_not_values() {
        let device = SimDevice::new();
        let secret = "hunter2-value";
        assert!(
            device
                .set("t", SimTag::new(PlcValue::String(secret.to_owned())))
                .is_ok(),
            "set"
        );
        let rendered = format!("{device:?}");
        assert!(
            rendered.contains("tags: 1"),
            "the count is shown: {rendered}"
        );
        assert!(!rendered.contains(secret), "the value is not: {rendered}");
        let tag = format!("{:?}", device.get("t"));
        assert!(!tag.contains(secret), "tag Debug hides the value: {tag}");
        assert!(tag.contains("String"), "but shows its type: {tag}");
    }

    // covers: SIM-DEVICE-MODEL (write)
    #[test]
    fn write_replaces_a_writable_value_of_the_same_type() {
        let device = SimDevice::new();
        device
            .set("t", SimTag::new(PlcValue::I16(1)))
            .expect("set t");
        device.write("t", PlcValue::I16(7)).expect("same type");
        assert_eq!(
            device.serve_read("t").ok(),
            Some(PlcValue::I16(7)),
            "replaced"
        );
        device
            .ramp("r", PlcValue::U8(0), PlcValue::U8(2))
            .expect("ramp");
        device
            .write("r", PlcValue::U8(10))
            .expect("ramp is writable");
        assert_eq!(
            device.serve_read("r").ok(),
            Some(PlcValue::U8(10)),
            "new start"
        );
        assert_eq!(
            device.serve_read("r").ok(),
            Some(PlcValue::U8(12)),
            "keeps stepping"
        );
    }

    // covers: SIM-DEVICE-MODEL (write refusals change nothing)
    #[test]
    fn write_refusals_are_typed_and_change_nothing() {
        let device = SimDevice::new();
        device
            .set("t", SimTag::new(PlcValue::I16(1)))
            .expect("set t");
        device
            .set("ro", SimTag::new(PlcValue::I16(2)).read_only())
            .expect("set ro");
        assert_eq!(
            kind(device.write("missing", PlcValue::I16(3))),
            Some(ErrorKind::InvalidAddress),
            "unset name"
        );
        assert_eq!(
            kind(device.write("ro", PlcValue::I16(3))),
            Some(ErrorKind::Unsupported(Operation::Write)),
            "not writable"
        );
        assert_eq!(
            kind(device.write("t", PlcValue::I32(3))),
            Some(ErrorKind::InvalidConfig),
            "other type"
        );
        assert_eq!(
            device.serve_read("t").ok(),
            Some(PlcValue::I16(1)),
            "t unchanged"
        );
        assert_eq!(
            device.serve_read("ro").ok(),
            Some(PlcValue::I16(2)),
            "ro unchanged"
        );
    }

    // covers: SIM-DEVICE-MODEL (SINK counter)
    #[test]
    fn sink_writes_are_counted() {
        let device = SimDevice::new();
        assert_eq!(device.sink_writes(), 0, "none yet");
        device.record_sink_write();
        device.record_sink_write();
        assert_eq!(device.sink_writes(), 2, "two counted");
        device.lock().sink_writes = u64::MAX;
        device.record_sink_write();
        assert_eq!(device.sink_writes(), u64::MAX, "the count saturates");
    }

    // covers: SIM-RANDOM-SEEDED (reseed)
    #[test]
    fn reseed_restarts_the_random_source_without_changing_the_reported_seed() {
        let expected = SimDevice::new()
            .with_seed(11)
            .random(PlcValueType::U64)
            .ok();
        let device = SimDevice::new();
        let before = device.random(PlcValueType::U64).ok();
        device.reseed(11);
        assert_eq!(
            device.random(PlcValueType::U64).ok(),
            expected,
            "same sequence as a device built with the seed"
        );
        assert_ne!(before, expected, "the default sequence differs");
        assert_eq!(
            device.seed(),
            DEFAULT_SEED,
            "seed() is the construction seed"
        );
        device.reseed(0);
        assert!(
            device.random(PlcValueType::U64).is_ok(),
            "zero reseeds to the constant"
        );
    }

    // covers: SIM-DEVICE-MODEL (poison recovery)
    #[test]
    fn a_panic_while_the_lock_is_held_does_not_break_the_device() {
        let device = Arc::new(SimDevice::new());
        assert!(device.set("t", SimTag::new(PlcValue::U8(3))).is_ok(), "set");
        let held = Arc::clone(&device);
        let worker = std::thread::spawn(move || {
            let _guard = held.lock();
            std::panic::resume_unwind(Box::new("poison the lock"));
        });
        assert!(
            worker.join().is_err(),
            "the worker panicked while holding the lock"
        );
        assert!(device.state.is_poisoned(), "the mutex is poisoned");
        assert_eq!(
            device.serve_read("t").ok(),
            Some(PlcValue::U8(3)),
            "reads recover"
        );
        assert!(
            device.set("u", SimTag::new(PlcValue::U8(4))).is_ok(),
            "writes recover"
        );
        assert_eq!(device.tags().len(), 2, "state is intact");
    }

    #[test]
    fn the_device_is_shareable_between_threads() {
        let device = Arc::new(SimDevice::new());
        assert!(
            device.ramp("n", PlcValue::U32(0), PlcValue::U32(1)).is_ok(),
            "ramp"
        );
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let shared = Arc::clone(&device);
                std::thread::spawn(move || {
                    (0..50).filter(|_| shared.serve_read("n").is_ok()).count()
                })
            })
            .collect();
        let total: usize = workers.into_iter().filter_map(|w| w.join().ok()).sum();
        assert_eq!(total, 200, "every read was served");
        assert_eq!(
            device.serve_read("n").ok(),
            Some(PlcValue::U32(200)),
            "the ramp saw all of them"
        );
    }
}
