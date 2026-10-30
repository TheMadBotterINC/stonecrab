/* SPDX-License-Identifier: Apache-2.0
 *
 * Stonecrab interoperability peer (slice S81): the check client of the image, on the client
 * of open62541 v1.5.9. It connects to the peer's server the way a test of the driver does,
 * with the policy, mode and identity named on the command line, reads the counter node and
 * prints it. The exit status is 0 only when the connect and the read both succeeded.
 *
 * First-party. Written from the public API of open62541 v1.5.9; none of its source is copied
 * here. Nothing is taken from Apache PLC4X.
 *
 *   client_check URL POLICY MODE [options]
 *     POLICY    None, Basic256Sha256, Aes128Sha256RsaOaep or Aes256Sha256RsaPss (the
 *               fragments Aes128_Sha256_RsaOaep and Aes256_Sha256_RsaPss of the policy URIs
 *               work as well)
 *     MODE      None, Sign or SignAndEncrypt
 *     --cert F --key F     the client's application certificate and key (DER)
 *     --app-uri URI        the client's application URI, default urn:stonecrab:interop:client
 *                          (it must be the URI alternative name of the client certificate)
 *     --trust F            a certificate the client trusts as the server's (DER). Without
 *                          one open62541 leaves its accept-all verification in place
 *                          ("Empty trustlist passed"), so a client with no list trusts every
 *                          server
 *     --user NAME --password-file F    a user name token
 *     --x509 CERT KEY      an X.509 token (DER)
 *     --read STRING        the string node id in namespace 1, default counter; the value is
 *                          printed in open62541's own text form
 *     --write-int32 N      write N, as an Int32, to the node before reading it back
 *     --session-timeout MS the requested session timeout, default 10000
 *     --hold-ms MS         wait this long (the client does not run, so it neither renews nor
 *                          pings) between the connect and the read
 *     --probe-events SEC   instead of the read: subscribe to the events of the node
 *                          events/Source for SEC seconds and print how many arrived
 *     --probe-endpoints    instead of the read: print the endpoints GetEndpoints returns and
 *                          the discovery URLs FindServers returns
 *     --probe-transfer     instead of the read: create a subscription on the counter, drop
 *                          the SecureChannel and keep the session, connect a second client
 *                          with the same options and send Republish and TransferSubscriptions
 *                          for the subscription, printing each service result */

#include <open62541/client_config_default.h>
#include <open62541/client_highlevel.h>
#include <open62541/client_subscriptions.h>
#include <open62541/plugin/log_stdout.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Exported by the library but declared only in its internal header: the generic synchronous
 * service call, chosen for brevity. A public asynchronous one, __UA_Client_AsyncService
 * (client_highlevel_async.h), would serve too and is less likely to change with the library. */
void __UA_Client_Service(UA_Client *client, const void *request, const UA_DataType *requestType,
                         void *response, const UA_DataType *responseType);

typedef struct {
    const char *url, *policy, *mode;
    const char *cert, *key, *trust, *app_uri;
    const char *user, *password_file, *x509_cert, *x509_key;
    UA_UInt32 session_timeout;
} Options;

static UA_ByteString
load_file(const char *path) {
    UA_ByteString out = UA_BYTESTRING_NULL;
    FILE *f = fopen(path, "rb");
    if(!f) {
        fprintf(stderr, "client_check: cannot open %s\n", path);
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

static int
usage(void) {
    fprintf(stderr, "usage: client_check URL POLICY MODE [--cert F --key F] [--app-uri URI] "
                    "[--trust F] [--user NAME --password-file F | --x509 CERT KEY] [--read ID] "
                    "[--write-int32 N] [--session-timeout MS] [--hold-ms MS] [--probe-transfer] [--probe-events SEC] [--probe-endpoints]\n");
    return 2;
}

/* open62541's text form of a value spreads over many lines; put it on one (and cut it). */
static void
print_one_line(UA_String *text) {
    for(size_t i = 0; i < text->length; i++)
        if(text->data[i] == '\n' || text->data[i] == '\t')
            text->data[i] = ' ';
    if(text->length > 400) /* a large array: the start of it is evidence enough */
        text->length = 400;
}

/* A configured client that has not connected yet; NULL when the options are unusable. */
static UA_Client *
new_client(const Options *o) {
    UA_MessageSecurityMode mode = UA_MESSAGESECURITYMODE_NONE;
    if(!strcmp(o->mode, "Sign")) mode = UA_MESSAGESECURITYMODE_SIGN;
    else if(!strcmp(o->mode, "SignAndEncrypt")) mode = UA_MESSAGESECURITYMODE_SIGNANDENCRYPT;

    UA_Client *client = UA_Client_new();
    UA_ClientConfig *cc = UA_Client_getConfig(client);
    UA_ByteString cert = o->cert ? load_file(o->cert) : UA_BYTESTRING_NULL;
    UA_ByteString key = o->key ? load_file(o->key) : UA_BYTESTRING_NULL;
    UA_ByteString trust = o->trust ? load_file(o->trust) : UA_BYTESTRING_NULL;
    UA_StatusCode rv = UA_ClientConfig_setDefaultEncryption(cc, cert, key, trust.length ? &trust : NULL,
                                                            trust.length ? 1 : 0, NULL, 0);
    UA_ByteString_clear(&cert);
    UA_ByteString_clear(&key);
    UA_ByteString_clear(&trust);
    if(rv != UA_STATUSCODE_GOOD) {
        fprintf(stderr, "client_check: encryption setup: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(client);
        return NULL;
    }
    UA_String_clear(&cc->clientDescription.applicationUri);
    cc->clientDescription.applicationUri = UA_STRING_ALLOC(o->app_uri);
    cc->requestedSessionTimeout = o->session_timeout;
    cc->securityMode = mode;
    UA_String_clear(&cc->securityPolicyUri);
    char uri[128];
    snprintf(uri, sizeof(uri), "http://opcfoundation.org/UA/SecurityPolicy#%s", o->policy);
    cc->securityPolicyUri = UA_STRING_ALLOC(uri);

    if(o->user && o->password_file) {
        UA_ByteString password = load_file(o->password_file);
        char *p = (char *)calloc(password.length + 1, 1);
        memcpy(p, password.data, password.length);
        rv = UA_ClientConfig_setAuthenticationUsername(cc, o->user, p);
        free(p);
        UA_ByteString_clear(&password);
    } else if(o->x509_cert) {
        UA_ByteString xcert = load_file(o->x509_cert), xkey = load_file(o->x509_key);
        rv = UA_ClientConfig_setAuthenticationCert(cc, xcert, xkey);
        UA_ByteString_clear(&xcert);
        UA_ByteString_clear(&xkey);
    }
    if(rv != UA_STATUSCODE_GOOD) {
        fprintf(stderr, "client_check: identity setup: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(client);
        return NULL;
    }
    return client;
}

static void
on_change(UA_Client *client, UA_UInt32 subId, void *subCtx, UA_UInt32 monId, void *monCtx,
          UA_DataValue *value) {
    (void)client; (void)subId; (void)subCtx; (void)monId; (void)monCtx; (void)value;
}

/* What the server advertises: its endpoints, and the discovery URLs of FindServers. */
static int
probe_endpoints(const Options *o) {
    UA_Client *c = new_client(o);
    if(!c)
        return 1;
    size_t n = 0, apps_size = 0;
    UA_EndpointDescription *eps = NULL;
    UA_ApplicationDescription *apps = NULL;
    UA_StatusCode rv = UA_Client_getEndpoints(c, o->url, &n, &eps);
    if(rv == UA_STATUSCODE_GOOD)
        rv = UA_Client_findServers(c, o->url, 0, NULL, 0, NULL, &apps_size, &apps);
    if(rv != UA_STATUSCODE_GOOD) {
        printf("endpoints: failed: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(c);
        return 1;
    }
    for(size_t i = 0; i < n; i++)
        printf("endpoint: %.*s mode=%d %.*s\n", (int)eps[i].endpointUrl.length, (const char *)eps[i].endpointUrl.data,
               (int)eps[i].securityMode, (int)eps[i].securityPolicyUri.length, (const char *)eps[i].securityPolicyUri.data);
    for(size_t i = 0; i < apps_size; i++)
        for(size_t j = 0; j < apps[i].discoveryUrlsSize; j++)
            printf("discovery: %.*s\n", (int)apps[i].discoveryUrls[j].length, (const char *)apps[i].discoveryUrls[j].data);
    printf("endpoints: %u\n", (unsigned)n);
    UA_Array_delete(eps, n, &UA_TYPES[UA_TYPES_ENDPOINTDESCRIPTION]);
    UA_Array_delete(apps, apps_size, &UA_TYPES[UA_TYPES_APPLICATIONDESCRIPTION]);
    UA_Client_delete(c);
    return 0;
}

static unsigned events_seen;

static void
on_event(UA_Client *client, UA_UInt32 subId, void *subCtx, UA_UInt32 monId, void *monCtx,
         const UA_KeyValueMap fields) {
    (void)client; (void)subId; (void)subCtx; (void)monId; (void)monCtx;
    events_seen++;
    for(size_t i = 0; i < fields.mapSize; i++) {
        UA_String text = UA_STRING_NULL;
        UA_print(&fields.map[i].value, &UA_TYPES[UA_TYPES_VARIANT], &text);
        print_one_line(&text);
        printf("event %u: %.*s = %.*s\n", events_seen, (int)fields.map[i].key.name.length,
               (const char *)fields.map[i].key.name.data, (int)text.length, (const char *)text.data);
        UA_String_clear(&text);
    }
}

/* Subscribe to the event source and count what arrives in the given seconds. */
static int
probe_events(const Options *o, int seconds) {
    UA_Client *c = new_client(o);
    if(!c)
        return 1;
    UA_StatusCode rv = UA_Client_connect(c, o->url);
    if(rv != UA_STATUSCODE_GOOD) {
        printf("connect: refused: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(c);
        return 1;
    }
    UA_CreateSubscriptionResponse sub =
        UA_Client_Subscriptions_create(c, UA_CreateSubscriptionRequest_default(), NULL, NULL, NULL);
    UA_MonitoredItemCreateRequest item;
    UA_MonitoredItemCreateRequest_init(&item);
    item.itemToMonitor.nodeId = UA_NODEID_STRING_ALLOC(1, "events/Source");
    item.itemToMonitor.attributeId = UA_ATTRIBUTEID_EVENTNOTIFIER;
    item.monitoringMode = UA_MONITORINGMODE_REPORTING;
    UA_EventFilter filter;
    rv = UA_EventFilter_parse(&filter, UA_STRING("SELECT /Message, /Severity"), NULL);
    UA_ExtensionObject_setValue(&item.requestedParameters.filter, &filter, &UA_TYPES[UA_TYPES_EVENTFILTER]);
    UA_MonitoredItemCreateResult mon = UA_Client_MonitoredItems_createEvent(
        c, sub.subscriptionId, UA_TIMESTAMPSTORETURN_BOTH, item, NULL, on_event, NULL);
    if(rv != UA_STATUSCODE_GOOD || mon.statusCode != UA_STATUSCODE_GOOD) {
        printf("events: subscription failed: %s\n", UA_StatusCode_name(mon.statusCode));
        UA_Client_delete(c);
        return 1;
    }
    for(int i = 0; i < seconds * 2; i++)
        UA_Client_run_iterate(c, 500);
    printf("events: %u\n", events_seen);
    UA_Client_delete(c);
    return events_seen > 0 ? 0 : 1;
}

/* Republish and TransferSubscriptions across two sessions of one application. */
static int
probe_transfer(const Options *o) {
    UA_Client *a = new_client(o);
    if(!a)
        return 1;
    UA_StatusCode rv = UA_Client_connect(a, o->url);
    if(rv != UA_STATUSCODE_GOOD) {
        printf("connect: refused: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(a);
        return 1;
    }
    UA_CreateSubscriptionResponse sub =
        UA_Client_Subscriptions_create(a, UA_CreateSubscriptionRequest_default(), NULL, NULL, NULL);
    UA_NodeId counter = UA_NODEID_STRING_ALLOC(1, "counter");
    UA_MonitoredItemCreateResult mon = UA_Client_MonitoredItems_createDataChange(
        a, sub.subscriptionId, UA_TIMESTAMPSTORETURN_BOTH, UA_MonitoredItemCreateRequest_default(counter),
        NULL, on_change, NULL);
    UA_NodeId_clear(&counter);
    if(sub.responseHeader.serviceResult != UA_STATUSCODE_GOOD || mon.statusCode != UA_STATUSCODE_GOOD) {
        printf("subscription: failed: %s / %s\n", UA_StatusCode_name(sub.responseHeader.serviceResult),
               UA_StatusCode_name(mon.statusCode));
        UA_Client_delete(a);
        return 1;
    }
    for(int i = 0; i < 6; i++)
        UA_Client_run_iterate(a, 500);

    /* Republish on the session that owns the subscription. Sequence number 1 was
     * acknowledged long ago by the client's next Publish, so the answer to expect is
     * BadMessageNotAvailable; BadServiceUnsupported would mean no support. */
    UA_RepublishRequest rep;
    UA_RepublishRequest_init(&rep);
    rep.subscriptionId = sub.subscriptionId;
    rep.retransmitSequenceNumber = 1;
    UA_RepublishResponse repr;
    __UA_Client_Service(a, &rep, &UA_TYPES[UA_TYPES_REPUBLISHREQUEST], &repr,
                        &UA_TYPES[UA_TYPES_REPUBLISHRESPONSE]);
    printf("republish: %s\n", UA_StatusCode_name(repr.responseHeader.serviceResult));
    UA_RepublishResponse_clear(&repr);

    /* Drop the channel, keep the session (and so the subscription) on the server. */
    UA_Client_disconnectSecureChannel(a);
    UA_Client_delete(a);

    UA_Client *b = new_client(o);
    if(!b)
        return 1;
    rv = UA_Client_connect(b, o->url);
    if(rv != UA_STATUSCODE_GOOD) {
        printf("connect: refused: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(b);
        return 1;
    }
    UA_TransferSubscriptionsRequest tr;
    UA_TransferSubscriptionsRequest_init(&tr);
    UA_UInt32 ids[1] = {sub.subscriptionId};
    tr.subscriptionIds = ids;
    tr.subscriptionIdsSize = 1;
    tr.sendInitialValues = true;
    UA_TransferSubscriptionsResponse trr;
    __UA_Client_Service(b, &tr, &UA_TYPES[UA_TYPES_TRANSFERSUBSCRIPTIONSREQUEST], &trr,
                        &UA_TYPES[UA_TYPES_TRANSFERSUBSCRIPTIONSRESPONSE]);
    UA_StatusCode status = trr.responseHeader.serviceResult;
    if(status == UA_STATUSCODE_GOOD && trr.resultsSize == 1)
        status = trr.results[0].statusCode;
    printf("transfer: %s", UA_StatusCode_name(status));
    if(trr.resultsSize == 1)
        printf(" (%u sequence numbers available)", (unsigned)trr.results[0].availableSequenceNumbersSize);
    printf("\n");
    UA_Boolean ok = status == UA_STATUSCODE_GOOD;
    UA_UInt32 first = (ok && trr.results[0].availableSequenceNumbersSize)
                          ? trr.results[0].availableSequenceNumbers[0] : 0;
    UA_TransferSubscriptionsResponse_clear(&trr);

    /* Republish a message the transfer reported as available, on the new session. */
    if(first) {
        rep.subscriptionId = sub.subscriptionId;
        rep.retransmitSequenceNumber = first;
        __UA_Client_Service(b, &rep, &UA_TYPES[UA_TYPES_REPUBLISHREQUEST], &repr,
                            &UA_TYPES[UA_TYPES_REPUBLISHRESPONSE]);
        printf("republish after transfer: %s\n", UA_StatusCode_name(repr.responseHeader.serviceResult));
        UA_RepublishResponse_clear(&repr);
    }
    UA_Client_delete(b);
    return ok ? 0 : 1;
}

int
main(int argc, char **argv) {
    if(argc < 4)
        return usage();
    Options o = {argv[1], argv[2], argv[3], NULL, NULL, NULL, "urn:stonecrab:interop:client",
                 NULL, NULL, NULL, NULL, 10000};
    const char *node = "counter";
    long hold_ms = 0;
    int transfer = 0, event_seconds = 0, write_set = 0, endpoints = 0;
    UA_Int32 write_value = 0;
    /* The URIs of the two newer policies carry underscores; accept the short names too. */
    if(!strcmp(o.policy, "Aes128Sha256RsaOaep")) o.policy = "Aes128_Sha256_RsaOaep";
    else if(!strcmp(o.policy, "Aes256Sha256RsaPss")) o.policy = "Aes256_Sha256_RsaPss";
    if(strcmp(o.mode, "None") && strcmp(o.mode, "Sign") && strcmp(o.mode, "SignAndEncrypt"))
        return usage();
    for(int i = 4; i < argc; i++) {
        if(!strcmp(argv[i], "--cert") && i + 1 < argc) o.cert = argv[++i];
        else if(!strcmp(argv[i], "--key") && i + 1 < argc) o.key = argv[++i];
        else if(!strcmp(argv[i], "--trust") && i + 1 < argc) o.trust = argv[++i];
        else if(!strcmp(argv[i], "--app-uri") && i + 1 < argc) o.app_uri = argv[++i];
        else if(!strcmp(argv[i], "--user") && i + 1 < argc) o.user = argv[++i];
        else if(!strcmp(argv[i], "--password-file") && i + 1 < argc) o.password_file = argv[++i];
        else if(!strcmp(argv[i], "--x509") && i + 2 < argc) { o.x509_cert = argv[++i]; o.x509_key = argv[++i]; }
        else if(!strcmp(argv[i], "--read") && i + 1 < argc) node = argv[++i];
        else if(!strcmp(argv[i], "--session-timeout") && i + 1 < argc) o.session_timeout = (UA_UInt32)atoi(argv[++i]);
        else if(!strcmp(argv[i], "--write-int32") && i + 1 < argc) { write_set = 1; write_value = atoi(argv[++i]); }
        else if(!strcmp(argv[i], "--hold-ms") && i + 1 < argc) hold_ms = atol(argv[++i]);
        else if(!strcmp(argv[i], "--probe-transfer")) transfer = 1;
        else if(!strcmp(argv[i], "--probe-endpoints")) endpoints = 1;
        else if(!strcmp(argv[i], "--probe-events") && i + 1 < argc) event_seconds = atoi(argv[++i]);
        else return usage();
    }
    if(transfer)
        return probe_transfer(&o);
    if(event_seconds > 0)
        return probe_events(&o, event_seconds);
    if(endpoints)
        return probe_endpoints(&o);

    UA_Client *client = new_client(&o);
    if(!client)
        return 1;
    UA_StatusCode rv = UA_Client_connect(client, o.url);
    if(rv != UA_STATUSCODE_GOOD) {
        printf("connect: refused: %s\n", UA_StatusCode_name(rv));
        UA_Client_delete(client);
        return 1;
    }
    if(hold_ms > 0)
        usleep((useconds_t)hold_ms * 1000);
    UA_Variant value;
    UA_Variant_init(&value);
    UA_NodeId id = UA_NODEID_STRING_ALLOC(1, node);
    if(write_set) {
        UA_Variant_setScalar(&value, &write_value, &UA_TYPES[UA_TYPES_INT32]);
        rv = UA_Client_writeValueAttribute(client, id, &value);
        printf("write: %s\n", UA_StatusCode_name(rv));
        UA_Variant_init(&value);
    }
    rv = UA_Client_readValueAttribute(client, id, &value);
    if(rv == UA_STATUSCODE_GOOD) {
        UA_String text = UA_STRING_NULL;
        UA_print(&value, &UA_TYPES[UA_TYPES_VARIANT], &text);
        print_one_line(&text);
        printf("read: %s = %.*s (%s %s)\n", node, (int)text.length, (const char *)text.data, o.policy, o.mode);
        UA_String_clear(&text);
    } else {
        printf("read: failed: %s\n", UA_StatusCode_name(rv));
    }
    UA_Variant_clear(&value);
    UA_NodeId_clear(&id);
    UA_Client_delete(client);
    return rv == UA_STATUSCODE_GOOD ? 0 : 1;
}
