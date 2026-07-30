/* What mechanisms does libsasl actually offer a client? Not what is on
 * disk — what it loaded and is willing to use.
 *
 * Two things have to be right or the answer is a misleading "no":
 *
 *  - Security properties. listmech hides mechanisms that put
 *    credentials on the wire in the clear, which is PLAIN, LOGIN and
 *    XOAUTH2. An IMAPS client declares an encrypted channel; so do we.
 *  - Callbacks. A mechanism that needs a username and a secret is not
 *    offered to a client that cannot supply them. With no callbacks at
 *    all you get EXTERNAL and ANONYMOUS and nothing else, which looks
 *    exactly like a missing plugin.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sasl/sasl.h>

static int get_simple(void *ctx, int id, const char **result, unsigned *len) {
    (void)ctx; (void)id;
    *result = "someone@example.test";
    if (len) *len = (unsigned)strlen(*result);
    return SASL_OK;
}

static int get_secret(sasl_conn_t *conn, void *ctx, int id, sasl_secret_t **psecret) {
    static sasl_secret_t *secret;
    const char *value = "not-a-real-token";
    (void)conn; (void)ctx; (void)id;
    if (!secret) {
        secret = malloc(sizeof(sasl_secret_t) + strlen(value) + 1);
        if (!secret) return SASL_NOMEM;
        secret->len = strlen(value);
        memcpy(secret->data, value, secret->len + 1);
    }
    *psecret = secret;
    return SASL_OK;
}

int main(void) {
    sasl_conn_t *conn = NULL;
    const char *list = NULL;
    sasl_security_properties_t sec;
    sasl_callback_t callbacks[] = {
        {SASL_CB_USER,     (int (*)(void))get_simple, NULL},
        {SASL_CB_AUTHNAME, (int (*)(void))get_simple, NULL},
        {SASL_CB_PASS,     (int (*)(void))get_secret, NULL},
        {SASL_CB_LIST_END, NULL, NULL},
    };
    if (sasl_client_init(callbacks) != SASL_OK) { puts("client_init failed"); return 2; }
    if (sasl_client_new("imap", "example.test", NULL, NULL, callbacks, 0, &conn) != SASL_OK) {
        puts("client_new failed"); return 2;
    }
    memset(&sec, 0, sizeof(sec));
    sec.max_ssf = 256;       /* as if already inside TLS */
    sec.security_flags = 0;  /* do not exclude plaintext mechanisms */
    if (sasl_setprop(conn, SASL_SEC_PROPS, &sec) != SASL_OK) { puts("setprop failed"); return 2; }
    if (sasl_listmech(conn, NULL, "", " ", "", &list, NULL, NULL) != SASL_OK) {
        puts("listmech failed"); return 2;
    }
    printf("mechs: %s\n", list);
    return strstr(list, "XOAUTH2") ? 0 : 1;
}
