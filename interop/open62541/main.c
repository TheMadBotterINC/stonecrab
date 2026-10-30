/* SPDX-License-Identifier: Apache-2.0
 *
 * Stonecrab interoperability peer (slice S81): an OPC UA server on open62541 v1.5.9.
 *
 * First-party. Written from the public API of open62541 v1.5.9 (the headers the image's
 * build installs); open62541 (MPL-2.0) is linked as a library of the image and none of its
 * source is copied here. Nothing is taken from Apache PLC4X.
 *
 * What it serves, in the namespace "urn:stonecrab:interop:open62541" (index 1; the namespace
 * URI is the application URI), all below the object ns=1;s=Interop:
 *
 *   scalar/<T>      every built-in type with a fixed sample value, read only
 *   writable/<T>    the same, initialized alike, read and write
 *   array/<T>       one-dimensional arrays of Boolean, Int32, Double and String, read only
 *   large/Int32_N   Int32 arrays of N = 4000 and 10000 elements (0..N-1), read only: 16 KB and
 *                   40 KB, to cross chunk boundaries
 *   matrix/...      Int32 2x3 (value rank 2) and Double 2x2x2 (value rank 3), read only
 *   writable/Int32Array   an Int32 array that takes any length
 *   counter         UInt32, +1 every second, read only
 *   structure       a custom structure (UInt32 id, Double value, String label), read only
 *   events/Source   an event source (BaseEventType, a tick every OPCUA_EVENT_PERIOD_MS)
 *   events/Fire     a method that fires one more event at once
 *
 * Configuration comes from the environment, so that serve.sh and the tests share one way:
 *
 *   OPCUA_PORT                 listen port, default 4840
 *   OPCUA_ADVERTISED_HOST      host of the endpoint URL the server advertises, default
 *                              localhost; with OPCUA_PORT it is the address a client reaches
 *                              (a mismatch with the reachable or proxied address hangs a
 *                              client's handshake)
 *   OPCUA_PKI_DIR              files made by pki.sh, default /pki
 *   OPCUA_TRUST                "pki" (default): the client certificate and the user
 *                              certificate are trusted; "empty": nothing is trusted
 *   OPCUA_SMALL_CHUNKS         1: 8192-byte chunks, at most 4 chunks and 24 KiB per message
 *                              (large/Int32_4000 still fits, large/Int32_10000 does not)
 *   OPCUA_MAX_SESSION_TIMEOUT_MS   session timeout ceiling, default 5000
 *   OPCUA_MAX_TOKEN_LIFETIME_MS    secure channel token ceiling, default 600000
 *   OPCUA_EVENT_PERIOD_MS      event tick, default 2000, 0 turns the tick off
 *   OPCUA_ALLOW_NONE_PASSWORD  1: accept a user name token on a None channel
 *
 * The user name is "stonecrab"; its password is the content of $OPCUA_PKI_DIR/user-password,
 * made at start by pki.sh. No key, certificate or password is in this file. */

#include <open62541/plugin/accesscontrol_default.h>
#include <open62541/plugin/certificategroup_default.h>
#include <open62541/plugin/log_stdout.h>
#include <open62541/server.h>
#include <open62541/server_config_default.h>

#include <signal.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define APP_URI "urn:stonecrab:interop:open62541"

static volatile UA_Boolean running = true;
static UA_UInt32 counter = 0;
static UA_UInt32 tick = 0;
static UA_NodeId eventSource;

static void
stopHandler(int sig) {
    (void)sig;
    running = false;
}

static const char *
env_str(const char *name, const char *fallback) {
    const char *v = getenv(name);
    return (v && *v) ? v : fallback;
}

static double
env_num(const char *name, double fallback) {
    const char *v = getenv(name);
    return (v && *v) ? atof(v) : fallback;
}

static UA_ByteString
load_file(const char *dir, const char *name) {
    char path[512];
    snprintf(path, sizeof(path), "%s/%s", dir, name);
    UA_ByteString out = UA_BYTESTRING_NULL;
    FILE *f = fopen(path, "rb");
    if(!f) {
        fprintf(stderr, "main: cannot open %s\n", path);
        return out;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if(n > 0 && UA_ByteString_allocBuffer(&out, (size_t)n) == UA_STATUSCODE_GOOD &&
       fread(out.data, 1, (size_t)n, f) != (size_t)n)
        UA_ByteString_clear(&out);
    fclose(f);
    return out;
}

/* A trust list holding the given certificates, for the application group or the user token
 * group. An empty list trusts nothing: the group still verifies, and refuses everything. */
static UA_StatusCode
set_trust(UA_CertificateGroup *group, UA_UInt32 groupId, const UA_ByteString *certs,
          size_t count, const UA_Logger *logger) {
    UA_TrustListDataType list;
    UA_TrustListDataType_init(&list);
    list.specifiedLists = UA_TRUSTLISTMASKS_TRUSTEDCERTIFICATES;
    list.trustedCertificates = (UA_ByteString *)(uintptr_t)certs;
    list.trustedCertificatesSize = count;
    UA_NodeId id = UA_NODEID_NUMERIC(0, groupId);
    return UA_CertificateGroup_Memorystore(group, &id, &list, logger, NULL);
}

/* ---- The test namespace ---------------------------------------------------------- */

typedef struct {
    UA_UInt32 id;
    UA_Double value;
    UA_String label;
} Reading;

static UA_DataTypeMember reading_members[3];
static UA_DataType reading_type;

static void
setup_reading_type(void) {
    reading_members[0] = (UA_DataTypeMember){UA_TYPENAME("Id") &UA_TYPES[UA_TYPES_UINT32], 0, false, false};
    reading_members[1] = (UA_DataTypeMember){
        UA_TYPENAME("Value") &UA_TYPES[UA_TYPES_DOUBLE],
        (UA_Byte)(offsetof(Reading, value) - offsetof(Reading, id) - sizeof(UA_UInt32)), false, false};
    reading_members[2] = (UA_DataTypeMember){
        UA_TYPENAME("Label") &UA_TYPES[UA_TYPES_STRING],
        (UA_Byte)(offsetof(Reading, label) - offsetof(Reading, value) - sizeof(UA_Double)), false, false};
    reading_type = (UA_DataType){UA_TYPENAME("Reading") {1, UA_NODEIDTYPE_NUMERIC, {5001}},
                                 {1, UA_NODEIDTYPE_NUMERIC, {5002}},
                                 {1, UA_NODEIDTYPE_NUMERIC, {5003}},
                                 sizeof(Reading),
                                 UA_DATATYPEKIND_STRUCTURE,
                                 false,
                                 false,
                                 3,
                                 reading_members};
}

static UA_NodeId
sid(const char *name) {
    return UA_NODEID_STRING_ALLOC(1, name);
}

static void
add_variable(UA_Server *server, const char *path, const UA_DataType *type, UA_Variant value,
             UA_Int32 rank, size_t dims_size, UA_UInt32 *dims, UA_Boolean writable) {
    UA_VariableAttributes attr = UA_VariableAttributes_default;
    attr.value = value;
    attr.dataType = type->typeId;
    attr.valueRank = rank;
    attr.arrayDimensionsSize = dims_size;
    attr.arrayDimensions = dims;
    attr.accessLevel = UA_ACCESSLEVELMASK_READ | (writable ? UA_ACCESSLEVELMASK_WRITE : 0);
    attr.displayName = UA_LOCALIZEDTEXT("en", (char *)(uintptr_t)path);
    UA_NodeId id = sid(path);
    UA_NodeId parent = sid("Interop");
    UA_StatusCode rv = UA_Server_addVariableNode(
        server, id, parent, UA_NS0ID(HASCOMPONENT), UA_QUALIFIEDNAME(1, (char *)(uintptr_t)path),
        UA_NS0ID(BASEDATAVARIABLETYPE), attr, NULL, NULL);
    if(rv != UA_STATUSCODE_GOOD)
        fprintf(stderr, "main: node %s not added: %s\n", path, UA_StatusCode_name(rv));
    UA_NodeId_clear(&id);
    UA_NodeId_clear(&parent);
}

static void
add_scalar(UA_Server *server, const char *name, const UA_DataType *type, void *sample) {
    char path[128];
    for(int writable = 0; writable < 2; writable++) {
        snprintf(path, sizeof(path), "%s/%s", writable ? "writable" : "scalar", name);
        UA_Variant v;
        UA_Variant_setScalar(&v, sample, type);
        add_variable(server, path, type, v, UA_VALUERANK_SCALAR, 0, NULL, writable);
    }
}

static void
add_array(UA_Server *server, const char *path, const UA_DataType *type, void *data, size_t n,
          UA_Boolean writable) {
    /* A fixed array reports its length as its dimension; the writable one takes any (0). */
    UA_UInt32 dim = writable ? 0 : (UA_UInt32)n;
    UA_Variant v;
    UA_Variant_setArray(&v, data, n, type);
    add_variable(server, path, type, v, UA_VALUERANK_ONE_DIMENSION, 1, &dim, writable);
}

static void
add_matrix(UA_Server *server, const char *path, const UA_DataType *type, void *data,
           size_t count, size_t rank, UA_UInt32 *dims) {
    UA_Variant v;
    UA_Variant_setArray(&v, data, count, type);
    v.arrayDimensions = dims;
    v.arrayDimensionsSize = rank;
    add_variable(server, path, type, v, (UA_Int32)rank, rank, dims, false);
}

static void
add_scalars(UA_Server *server) {
    UA_Boolean boolean = true;
    UA_SByte sbyte = -12;
    UA_Byte byte = 200;
    UA_Int16 int16 = -1234;
    UA_UInt16 uint16 = 54321;
    UA_Int32 int32 = -123456789;
    UA_UInt32 uint32 = 3123456789u;
    UA_Int64 int64 = -1234567890123LL;
    UA_UInt64 uint64 = 12345678901234567890ULL;
    UA_Float flt = 1.5f;
    UA_Double dbl = 2.718281828459045;
    UA_String string = UA_STRING("stonecrab");
    UA_DateTime date = UA_DATETIME_UNIX_EPOCH + 1793318400LL * UA_DATETIME_SEC; /* 2026-10-30 */
    UA_Guid guid;
    UA_Guid_parse(&guid, UA_STRING("72962B91-FA75-4AE6-8D28-B404DC7DAF63"));
    UA_Byte bytes[4] = {0x00, 0x01, 0xFE, 0xFF};
    UA_ByteString bytestring = {4, bytes};
    UA_XmlElement xml = UA_STRING("<a>1</a>");
    UA_NodeId node = UA_NODEID_NUMERIC(0, 2253);
    UA_ExpandedNodeId expanded = UA_EXPANDEDNODEID_STRING(1, "counter");
    UA_StatusCode status = UA_STATUSCODE_BADTIMEOUT;
    UA_QualifiedName qname = UA_QUALIFIEDNAME(1, "qname");
    UA_LocalizedText text = UA_LOCALIZEDTEXT("en-US", "text");
    UA_Int32 inner = 7;
    UA_DataValue dv;
    UA_DataValue_init(&dv);
    UA_Variant_setScalar(&dv.value, &inner, &UA_TYPES[UA_TYPES_INT32]);
    dv.hasValue = true;
    dv.status = UA_STATUSCODE_UNCERTAINLASTUSABLEVALUE;
    dv.hasStatus = true;
    UA_DiagnosticInfo di;
    UA_DiagnosticInfo_init(&di);
    di.hasAdditionalInfo = true;
    di.additionalInfo = UA_STRING("diag");
    Reading reading = {42, 0.25, UA_STRING("sample")};

    add_scalar(server, "Boolean", &UA_TYPES[UA_TYPES_BOOLEAN], &boolean);
    add_scalar(server, "SByte", &UA_TYPES[UA_TYPES_SBYTE], &sbyte);
    add_scalar(server, "Byte", &UA_TYPES[UA_TYPES_BYTE], &byte);
    add_scalar(server, "Int16", &UA_TYPES[UA_TYPES_INT16], &int16);
    add_scalar(server, "UInt16", &UA_TYPES[UA_TYPES_UINT16], &uint16);
    add_scalar(server, "Int32", &UA_TYPES[UA_TYPES_INT32], &int32);
    add_scalar(server, "UInt32", &UA_TYPES[UA_TYPES_UINT32], &uint32);
    add_scalar(server, "Int64", &UA_TYPES[UA_TYPES_INT64], &int64);
    add_scalar(server, "UInt64", &UA_TYPES[UA_TYPES_UINT64], &uint64);
    add_scalar(server, "Float", &UA_TYPES[UA_TYPES_FLOAT], &flt);
    add_scalar(server, "Double", &UA_TYPES[UA_TYPES_DOUBLE], &dbl);
    add_scalar(server, "String", &UA_TYPES[UA_TYPES_STRING], &string);
    add_scalar(server, "DateTime", &UA_TYPES[UA_TYPES_DATETIME], &date);
    add_scalar(server, "Guid", &UA_TYPES[UA_TYPES_GUID], &guid);
    add_scalar(server, "ByteString", &UA_TYPES[UA_TYPES_BYTESTRING], &bytestring);
    add_scalar(server, "XmlElement", &UA_TYPES[UA_TYPES_XMLELEMENT], &xml);
    add_scalar(server, "NodeId", &UA_TYPES[UA_TYPES_NODEID], &node);
    add_scalar(server, "ExpandedNodeId", &UA_TYPES[UA_TYPES_EXPANDEDNODEID], &expanded);
    add_scalar(server, "StatusCode", &UA_TYPES[UA_TYPES_STATUSCODE], &status);
    add_scalar(server, "QualifiedName", &UA_TYPES[UA_TYPES_QUALIFIEDNAME], &qname);
    add_scalar(server, "LocalizedText", &UA_TYPES[UA_TYPES_LOCALIZEDTEXT], &text);
    add_scalar(server, "DataValue", &UA_TYPES[UA_TYPES_DATAVALUE], &dv);
    add_scalar(server, "DiagnosticInfo", &UA_TYPES[UA_TYPES_DIAGNOSTICINFO], &di);

    UA_Variant v;
    UA_Variant_setScalar(&v, &reading, &reading_type);
    add_variable(server, "structure", &reading_type, v, UA_VALUERANK_SCALAR, 0, NULL, false);

    UA_Boolean bools[3] = {true, false, true};
    UA_Int32 ints[5] = {1, -2, 3, -4, 5};
    UA_Double dbls[3] = {0.5, 1.5, 2.5};
    UA_String strs[3] = {UA_STRING("alpha"), UA_STRING("beta"), UA_STRING("gamma")};
    UA_Int32 open[2] = {0, 0};
    add_array(server, "array/Boolean", &UA_TYPES[UA_TYPES_BOOLEAN], bools, 3, false);
    add_array(server, "array/Int32", &UA_TYPES[UA_TYPES_INT32], ints, 5, false);
    add_array(server, "array/Double", &UA_TYPES[UA_TYPES_DOUBLE], dbls, 3, false);
    add_array(server, "array/String", &UA_TYPES[UA_TYPES_STRING], strs, 3, false);
    add_array(server, "writable/Int32Array", &UA_TYPES[UA_TYPES_INT32], open, 2, true);

    /* 16 KB and 40 KB: several 8192-byte chunks, and more than the small limits allow. */
    for(size_t n = 4000; n <= 10000; n += 6000) {
        char path[32];
        UA_Int32 *big = (UA_Int32 *)UA_Array_new(n, &UA_TYPES[UA_TYPES_INT32]);
        for(size_t i = 0; big && i < n; i++)
            big[i] = (UA_Int32)i;
        snprintf(path, sizeof(path), "large/Int32_%u", (unsigned)n);
        add_array(server, path, &UA_TYPES[UA_TYPES_INT32], big, big ? n : 0, false);
        UA_Array_delete(big, n, &UA_TYPES[UA_TYPES_INT32]);
    }

    UA_Int32 m23[6] = {1, 2, 3, 4, 5, 6};
    UA_UInt32 d23[2] = {2, 3};
    UA_Double m222[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    UA_UInt32 d222[3] = {2, 2, 2};
    add_matrix(server, "matrix/Int32_2x3", &UA_TYPES[UA_TYPES_INT32], m23, 6, 2, d23);
    add_matrix(server, "matrix/Double_2x2x2", &UA_TYPES[UA_TYPES_DOUBLE], m222, 8, 3, d222);

    UA_Variant c;
    UA_Variant_setScalar(&c, &counter, &UA_TYPES[UA_TYPES_UINT32]);
    add_variable(server, "counter", &UA_TYPES[UA_TYPES_UINT32], c, UA_VALUERANK_SCALAR, 0, NULL, false);
}

static void
count_up(UA_Server *server, void *data) {
    (void)data;
    counter++;
    UA_Variant v;
    UA_Variant_setScalar(&v, &counter, &UA_TYPES[UA_TYPES_UINT32]);
    UA_NodeId id = sid("counter");
    UA_Server_writeValue(server, id, v);
    UA_NodeId_clear(&id);
}

static void
fire_event(UA_Server *server, const char *what) {
    char text[64];
    snprintf(text, sizeof(text), "%s %u", what, (unsigned)++tick);
    UA_Server_createEvent(server, eventSource, UA_NS0ID(BASEEVENTTYPE), 500,
                          UA_LOCALIZEDTEXT("en-US", text), NULL, NULL, NULL);
}

static void
event_tick(UA_Server *server, void *data) {
    (void)data;
    fire_event(server, "tick");
}

static UA_StatusCode
fire_method(UA_Server *server, const UA_NodeId *session, void *sessionCtx, const UA_NodeId *method,
            void *methodCtx, const UA_NodeId *object, void *objectCtx, size_t inSize,
            const UA_Variant *in, size_t outSize, UA_Variant *out) {
    (void)session; (void)sessionCtx; (void)method; (void)methodCtx; (void)object;
    (void)objectCtx; (void)inSize; (void)in; (void)outSize; (void)out;
    fire_event(server, "fired");
    return UA_STATUSCODE_GOOD;
}

static void
add_events(UA_Server *server, double period_ms) {
    UA_ObjectAttributes attr = UA_ObjectAttributes_default;
    attr.displayName = UA_LOCALIZEDTEXT("en", "events/Source");
    attr.eventNotifier = UA_EVENTNOTIFIER_SUBSCRIBE_TO_EVENT;
    UA_Server_addObjectNode(server, eventSource, sid("Interop"), UA_NS0ID(HASCOMPONENT),
                            UA_QUALIFIEDNAME(1, "events/Source"), UA_NS0ID(BASEOBJECTTYPE), attr,
                            NULL, NULL);
    /* So that a subscription on the Server object sees the events too. */
    UA_Server_addReference(server, UA_NS0ID(SERVER), UA_NS0ID(HASEVENTSOURCE),
                           UA_EXPANDEDNODEID_NODEID(eventSource), true);

    UA_MethodAttributes m = UA_MethodAttributes_default;
    m.displayName = UA_LOCALIZEDTEXT("en", "events/Fire");
    m.executable = true;
    m.userExecutable = true;
    UA_Server_addMethodNode(server, sid("events/Fire"), sid("Interop"), UA_NS0ID(HASCOMPONENT),
                            UA_QUALIFIEDNAME(1, "events/Fire"), m, fire_method, 0, NULL, 0, NULL,
                            NULL, NULL);
    if(period_ms > 0)
        UA_Server_addRepeatedCallback(server, event_tick, NULL, period_ms, NULL);
}

/* ---- Configuration --------------------------------------------------------------- */

static UA_StatusCode
configure(UA_ServerConfig *config, const char *pki, UA_Boolean trust_pki) {
    UA_UInt16 port = (UA_UInt16)env_num("OPCUA_PORT", 4840);
    UA_StatusCode rv = UA_ServerConfig_setBasics_withPort(config, port);
    if(rv != UA_STATUSCODE_GOOD)
        return rv;

    /* Identity. The server certificate's URI alternative name is this application URI. */
    UA_String_clear(&config->applicationDescription.applicationUri);
    config->applicationDescription.applicationUri = UA_STRING_ALLOC(APP_URI);
    UA_LocalizedText_clear(&config->applicationDescription.applicationName);
    config->applicationDescription.applicationName = UA_LOCALIZEDTEXT_ALLOC("en", "Stonecrab open62541 peer");

    /* The advertised address: what a client (or a proxy in front of the server) reaches. */
    char url[300];
    snprintf(url, sizeof(url), "opc.tcp://%s:%u", env_str("OPCUA_ADVERTISED_HOST", "localhost"), (unsigned)port);
    UA_String discovery = UA_STRING(url);
    UA_Array_appendCopy((void **)&config->applicationDescription.discoveryUrls,
                        &config->applicationDescription.discoveryUrlsSize, &discovery,
                        &UA_TYPES[UA_TYPES_STRING]);

    /* Limits. */
    config->maxSessionTimeout = env_num("OPCUA_MAX_SESSION_TIMEOUT_MS", 5000);
    config->maxSecurityTokenLifetime = (UA_UInt32)env_num("OPCUA_MAX_TOKEN_LIFETIME_MS", 600000);
    if(getenv("OPCUA_SMALL_CHUNKS") && atoi(getenv("OPCUA_SMALL_CHUNKS")) == 1) {
        config->tcpBufSize = 8192;
        config->tcpMaxChunks = 4;
        config->tcpMaxMsgSize = 24576;
    }
    config->allowNonePolicyPassword = getenv("OPCUA_ALLOW_NONE_PASSWORD") && atoi(getenv("OPCUA_ALLOW_NONE_PASSWORD")) == 1;

    /* Trust: the client application certificate for the channel, the user certificate for
     * the X.509 token. Set before the policies, as open62541 asks. */
    UA_ByteString client = trust_pki ? load_file(pki, "client.cert.der") : UA_BYTESTRING_NULL;
    UA_ByteString user = trust_pki ? load_file(pki, "user.cert.der") : UA_BYTESTRING_NULL;
    if(trust_pki && (!client.length || !user.length))
        return UA_STATUSCODE_BADINTERNALERROR;
    rv = set_trust(&config->secureChannelPKI, UA_NS0ID_SERVERCONFIGURATION_CERTIFICATEGROUPS_DEFAULTAPPLICATIONGROUP,
                   &client, trust_pki ? 1 : 0, config->logging);
    if(rv == UA_STATUSCODE_GOOD)
        rv = set_trust(&config->sessionPKI, UA_NS0ID_SERVERCONFIGURATION_CERTIFICATEGROUPS_DEFAULTUSERTOKENGROUP,
                       &user, trust_pki ? 1 : 0, config->logging);
    UA_ByteString_clear(&client);
    UA_ByteString_clear(&user);
    if(rv != UA_STATUSCODE_GOOD)
        return rv;

    /* Policies: None and the three that are not deprecated. */
    UA_ByteString cert = load_file(pki, "server.cert.der");
    UA_ByteString key = load_file(pki, "server.key.der");
    if(!cert.length || !key.length)
        return UA_STATUSCODE_BADINTERNALERROR;
    rv = UA_ServerConfig_addSecurityPolicyNone(config, &cert);
    if(rv == UA_STATUSCODE_GOOD)
        rv = UA_ServerConfig_addSecurityPolicyBasic256Sha256(config, &cert, &key);
    if(rv == UA_STATUSCODE_GOOD)
        rv = UA_ServerConfig_addSecurityPolicyAes128Sha256RsaOaep(config, &cert, &key);
    if(rv == UA_STATUSCODE_GOOD)
        rv = UA_ServerConfig_addSecurityPolicyAes256Sha256RsaPss(config, &cert, &key);
    UA_ByteString_clear(&cert);
    UA_ByteString_memZero(&key);
    UA_ByteString_clear(&key);
    if(rv == UA_STATUSCODE_GOOD)
        rv = UA_ServerConfig_addAllEndpoints(config);
    if(rv != UA_STATUSCODE_GOOD)
        return rv;

    /* Identity tokens: anonymous, user name, X.509 (offered because sessionPKI is set). */
    UA_ByteString password = load_file(pki, "user-password");
    UA_UsernamePasswordLogin login = {UA_STRING("stonecrab"), password};
    rv = UA_AccessControl_default(config, true, NULL, 1, &login);
    UA_ByteString_memZero(&password);
    UA_ByteString_clear(&password);
    if(rv != UA_STATUSCODE_GOOD)
        return rv;

    /* The structure type. */
    setup_reading_type();
    static UA_DataTypeArray custom = {NULL, 1, &reading_type, false};
    config->customDataTypes = &custom;
    return UA_STATUSCODE_GOOD;
}

int
main(void) {
    signal(SIGINT, stopHandler);
    signal(SIGTERM, stopHandler);
    const char *pki = env_str("OPCUA_PKI_DIR", "/pki");
    UA_Boolean trust_pki = strcmp(env_str("OPCUA_TRUST", "pki"), "empty") != 0;

    UA_ServerConfig config;
    memset(&config, 0, sizeof(config));
    UA_StatusCode rv = configure(&config, pki, trust_pki);
    if(rv != UA_STATUSCODE_GOOD) {
        fprintf(stderr, "main: configuration failed: %s\n", UA_StatusCode_name(rv));
        UA_ServerConfig_clear(&config);
        return EXIT_FAILURE;
    }
    UA_Server *server = UA_Server_newWithConfig(&config);
    if(!server)
        return EXIT_FAILURE;
    eventSource = UA_NODEID_STRING_ALLOC(1, "events/Source");

    /* Namespace 1 is the application URI. The server adds the node of the structure type
     * itself, from the custom data types of the configuration. */
    UA_ObjectAttributes folder = UA_ObjectAttributes_default;
    folder.displayName = UA_LOCALIZEDTEXT("en", "Interop");
    UA_Server_addObjectNode(server, sid("Interop"), UA_NS0ID(OBJECTSFOLDER), UA_NS0ID(ORGANIZES),
                            UA_QUALIFIEDNAME(1, "Interop"), UA_NS0ID(BASEOBJECTTYPE), folder, NULL, NULL);
    add_scalars(server);
    add_events(server, env_num("OPCUA_EVENT_PERIOD_MS", 2000));
    UA_Server_addRepeatedCallback(server, count_up, NULL, 1000.0, NULL);

    rv = UA_Server_run(server, &running);
    UA_Server_delete(server);
    UA_NodeId_clear(&eventSource);
    return rv == UA_STATUSCODE_GOOD ? EXIT_SUCCESS : EXIT_FAILURE;
}
