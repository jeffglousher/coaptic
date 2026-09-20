#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
/* Independent libcoap fixture. No Coaptic codecs or library code. */
#include <coap3/coap.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <time.h>
#endif

static unsigned counter;
static uint8_t method_body[64];
static size_t method_length;
static int method_exists;
static uint8_t large_body[2000];
static const uint8_t small_body[] = "core-test-payload";
static int complete;
static uint64_t started;
static uint64_t clock_resolution_ns;
#ifdef _WIN32
static uint64_t clock_frequency;
#define CLOCK_NAME "QueryPerformanceCounter"
#else
#define CLOCK_NAME "CLOCK_MONOTONIC"
#endif


static void failure(const char *message) {
  /* Only fixed diagnostics are passed, never unescaped external strings. */
  printf("{\"schema\":\"coaptic-peer/2\",\"event\":\"error\",\"message\":\"%s\"}\n", message);
}
/* Measure intervals with the OS monotonic clock, independently of libcoap's
 * protocol tick granularity. Capture before payload-to-JSON formatting. */
static void clock_failure(void) {
  failure("monotonic clock unavailable");
  exit(1);
}
static void clock_init(void) {
#ifdef _WIN32
  LARGE_INTEGER frequency;
  if (!QueryPerformanceFrequency(&frequency) || frequency.QuadPart <= 0) clock_failure();
  clock_frequency = (uint64_t)frequency.QuadPart;
  clock_resolution_ns = 1000000000ULL / clock_frequency + (1000000000ULL % clock_frequency != 0);
#else
  struct timespec resolution;
  if (clock_getres(CLOCK_MONOTONIC, &resolution) != 0) clock_failure();
  clock_resolution_ns = (uint64_t)resolution.tv_sec * 1000000000ULL + (uint64_t)resolution.tv_nsec;
#endif
}
static uint64_t clock_stamp(void) {
#ifdef _WIN32
  LARGE_INTEGER value;
  if (!QueryPerformanceCounter(&value)) clock_failure();
  return (uint64_t)value.QuadPart;
#else
  struct timespec value;
  if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) clock_failure();
  return (uint64_t)value.tv_sec * 1000000000ULL + (uint64_t)value.tv_nsec;
#endif
}
static uint64_t elapsed_ns(void) {
  uint64_t now = clock_stamp();
  if (now < started) clock_failure();
  uint64_t delta = now - started;
#ifdef _WIN32
  return (delta / clock_frequency) * 1000000000ULL +
      (uint64_t)((long double)(delta % clock_frequency) * 1000000000.0L / clock_frequency);
#else
  return delta;
#endif
}
static void get_fixture(coap_resource_t *resource, coap_session_t *session,
                        const coap_pdu_t *request, const coap_string_t *query,
                        coap_pdu_t *response) {
  const coap_str_const_t *path = coap_resource_get_uri_path(resource);
  const uint8_t *body = small_body;
  size_t len = sizeof(small_body)-1;
  char count[32];
  if (path->length == 5 && memcmp(path->s,"large",5)==0) {
    body=large_body; len=sizeof(large_body);
  } else if (path->length == 7 && memcmp(path->s,"counter",7)==0) {
    len=(size_t)snprintf(count,sizeof(count),"%u",counter);body=(const uint8_t *)count;
  }
  coap_pdu_set_code(response,COAP_RESPONSE_CODE_CONTENT);
  if (body == (const uint8_t *)count) {
    coap_add_data(response,len,body);
  } else {
    coap_add_data_large_response(resource,session,request,response,query,
        COAP_MEDIATYPE_APPLICATION_OCTET_STREAM,60,0,len,body,NULL,NULL);
  }
}
static void post_counter(coap_resource_t *resource, coap_session_t *session,
                        const coap_pdu_t *request, const coap_string_t *query,
                        coap_pdu_t *response) {
  (void)resource;(void)session;(void)request;(void)query;
  counter++;coap_pdu_set_code(response,COAP_RESPONSE_CODE_CHANGED);
}
/* Bounded application workflow fixture; each CoAP stack owns its wire parsing. */
static void method_resource(coap_resource_t *resource, coap_session_t *session,
    const coap_pdu_t *request, const coap_string_t *query, coap_pdu_t *response) {
  (void)resource; (void)session; (void)query;
  unsigned method = (unsigned)coap_pdu_get_code(request);
  size_t len = 0; const uint8_t *data = NULL;
  coap_get_data(request, &len, &data);
  unsigned code = 68;
  if (method == 2 || method == 3 || method == 5 || method == 6 || method == 7) {
    coap_opt_iterator_t it;
    coap_opt_t *format = coap_check_option(request, COAP_OPTION_CONTENT_FORMAT, &it);
    if (!format || coap_opt_length(format) > 2 || coap_decode_var_bytes(coap_opt_value(format), coap_opt_length(format)) != 42) {
      coap_pdu_set_code(response, COAP_RESPONSE_CODE_UNSUPPORTED_CONTENT_FORMAT); return;
    }
  }
  if (len > sizeof(method_body)) { coap_pdu_set_code(response, COAP_RESPONSE_CODE_REQUEST_TOO_LARGE); return; }
  if (method == 1 || method == 5) {
    if (method == 5 && (len != 5 || memcmp(data, "value", 5))) code = 128;
    else if (!method_exists) code = 132;
    else { code = 69; if (method_length) coap_add_data(response, method_length, method_body); }
  } else if (method == 3) {
    code = method_exists ? 68 : 65;
    if (len) memcpy(method_body, data, len);
    method_length = len; method_exists = 1;
  } else if (method == 4) {
    code = method_exists ? 66 : 132;
    method_exists = 0; method_length = 0;
  } else if (method == 2 || method == 6 || method == 7) {
    if (!method_exists) code = 132;
    else if ((method == 6 && (!len || data[0] != '+')) || (method == 7 && (!len || data[0] != '='))) code = 128;
    else {
      size_t skip = method == 2 ? 0 : 1;
      size_t keep = method == 7 ? 0 : method_length;
      if (keep + len - skip > sizeof(method_body)) code = 141;
      else {
        if (len > skip) memcpy(method_body + keep, data + skip, len - skip);
        method_length = keep + len - skip;
      }
    }
  } else code = 133;
  coap_pdu_set_code(response, (coap_pdu_code_t)code);
}
static int hex_digit(char c) {
  if(c >= '0' && c <= '9') return c - '0';
  if(c >= 'a' && c <= 'f') return c - 'a' + 10;
  if(c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}
static coap_response_t response_handler(coap_session_t *session,
    const coap_pdu_t *sent,const coap_pdu_t *received,const coap_mid_t mid) {
  (void)session;(void)sent;(void)mid;
  const uint8_t *data=NULL;size_t len=0,offset=0,total=0;
  coap_get_data_large(received,&len,&data,&offset,&total);
  if (offset || total>len) return COAP_RESPONSE_FAIL;
  uint64_t elapsed = elapsed_ns();
  printf("{\"schema\":\"coaptic-peer/2\",\"event\":\"response\",\"code\":%u,\"payload_hex\":\"",(unsigned)coap_pdu_get_code(received));
  for(size_t i=0;i<len;i++)printf("%02x",data[i]);
  printf("\",\"elapsed_ns\":%llu,\"elapsed_us\":%.3f,\"clock\":{\"name\":\"%s\",\"resolution_ns\":%llu}}\n",
      (unsigned long long)elapsed, (double)elapsed / 1000.0, CLOCK_NAME,
      (unsigned long long)clock_resolution_ns);
  complete=1;return COAP_RESPONSE_OK;
}
int main(int argc,char **argv) {
  setvbuf(stdout,NULL,_IONBF,0);
  if(argc<8 || argc>10){failure("invalid arguments");return 2;}
  const char *family = argc >= 9 ? argv[8] : "ipv4";
  if(strcmp(family,"ipv4") && strcmp(family,"ipv6")){failure("invalid address family");return 2;}
  const int ipv6 = strcmp(family,"ipv6")==0;
  const int server=strcmp(argv[1],"server")==0,dtls=strcmp(argv[2],"dtls")==0;
  if ((!server && strcmp(argv[1],"client")) || (!dtls && strcmp(argv[2],"udp"))) {failure("invalid mode");return 2;}
  char *end=NULL;long port=strtol(argv[3],&end,10);
  if(*end || port<1 || port>65535){failure("invalid port");return 2;}
  long timeout=strtol(argv[7],&end,10);
  if(*end || timeout<100 || timeout>30000){failure("invalid timeout");return 2;}
  if(strcmp(argv[5],"test") && strcmp(argv[5],"large") && strcmp(argv[5],"counter") && strcmp(argv[5],"missing") && strcmp(argv[5],"methods")){failure("unsupported path");return 2;}
  const char *methods[] = {"GET", "POST", "PUT", "DELETE", "FETCH", "PATCH", "IPATCH"};
  unsigned method = 0;
  for(unsigned i = 0; i < 7; i++) if(!strcmp(argv[6], methods[i])) method = i + 1;
  if(!method) { failure("unsupported method"); return 2; }
  const char *hex = argc == 10 ? argv[9] : "";
  size_t payload_length = strlen(hex) / 2;
  uint8_t payload[256];
  if(strlen(hex) % 2 || payload_length > sizeof(payload)) { failure("invalid bounded payload hex"); return 2; }
  for(size_t i = 0; i < payload_length; i++) {
    int hi = hex_digit(hex[i*2]), lo = hex_digit(hex[i*2+1]);
    if(hi < 0 || lo < 0) { failure("invalid bounded payload hex"); return 2; }
    payload[i] = (uint8_t)(hi*16 + lo);
  }
  if(strlen(argv[4])<1 || strlen(argv[4])>64){failure("invalid PSK length");return 2;}
  clock_init();started=clock_stamp();
  coap_startup();coap_set_log_level(COAP_LOG_EMERG);
  if(dtls && !coap_dtls_is_supported()){failure("DTLS unavailable in libcoap build");coap_cleanup();return 2;}
  for(size_t i=0;i<sizeof(large_body);i++)large_body[i]=(uint8_t)(i%251);
  coap_context_t *ctx=coap_new_context(NULL);
  if(!ctx){failure("context failed");coap_cleanup();return 1;}
  coap_context_set_block_mode(ctx,COAP_BLOCK_USE_LIBCOAP|COAP_BLOCK_SINGLE_BODY);
  coap_address_t addr;coap_address_init(&addr);
  if(ipv6) {
    addr.addr.sin6.sin6_family=AF_INET6;
    addr.addr.sin6.sin6_addr.s6_addr[15]=1;
    addr.addr.sin6.sin6_port=htons((uint16_t)port);
    addr.size=sizeof(addr.addr.sin6);
  } else {
    addr.addr.sin.sin_family=AF_INET;addr.addr.sin.sin_addr.s_addr=htonl(INADDR_LOOPBACK);addr.addr.sin.sin_port=htons((uint16_t)port);addr.size=sizeof(addr.addr.sin);
  }
  coap_proto_t proto=dtls?COAP_PROTO_DTLS:COAP_PROTO_UDP;
  int status=0;
  if(server) {
    if(dtls && !coap_context_set_psk(ctx,"password",(const uint8_t *)argv[4],(unsigned)strlen(argv[4]))){failure("PSK setup failed");status=1;goto done;}
    if(!coap_new_endpoint(ctx,&addr,proto)){failure("bind failed");status=1;goto done;}
    const char *paths[]={"test","large","counter"};
    for(size_t i=0;i<3;i++) {
      coap_resource_t *r=coap_resource_init(coap_make_str_const(paths[i]),0);
      coap_register_handler(r,COAP_REQUEST_GET,get_fixture);
      if(i==2)coap_register_handler(r,COAP_REQUEST_POST,post_counter);
      coap_add_resource(ctx,r);
    }
    coap_resource_t *methods_resource = coap_resource_init(coap_make_str_const("methods"), 0);
    for(unsigned i = 1; i <= 7; i++) coap_register_handler(methods_resource, (coap_request_t)i, method_resource);
    coap_add_resource(ctx, methods_resource);
    printf("{\"schema\":\"coaptic-peer/2\",\"event\":\"ready\",\"peer\":\"libcoap\",\"stack\":\"libcoap %s\",\"port\":%ld,\"transport\":\"%s\"}\n",LIBCOAP_PACKAGE_VERSION,port,dtls?"dtls":"udp");
    while(coap_io_process(ctx,100)>=0) {}
    status=1;
  } else {
    coap_session_t *session=dtls?coap_new_client_session_psk(ctx,NULL,&addr,proto,"password",(const uint8_t *)argv[4],(unsigned)strlen(argv[4])):coap_new_client_session(ctx,NULL,&addr,proto);
    if(!session){failure("session failed");status=1;goto done;}
    coap_register_response_handler(ctx,response_handler);
    coap_pdu_t *pdu=coap_new_pdu(COAP_MESSAGE_CON,(coap_pdu_code_t)method,session);
    if(!pdu){failure("PDU allocation failed");coap_session_release(session);status=1;goto done;}
    uint8_t token[8];size_t token_len=sizeof(token);coap_session_new_token(session,&token_len,token);
    if(!coap_add_token(pdu,token_len,token)||!coap_add_option(pdu,COAP_OPTION_URI_PATH,strlen(argv[5]),(const uint8_t *)argv[5])){coap_delete_pdu(pdu);failure("PDU construction failed");coap_session_release(session);status=1;goto done;}
    if(!strcmp(argv[5], "methods") && (method == 2 || method == 3 || method == 5 || method == 6 || method == 7)) {
      const uint8_t format = 42;
      if(!coap_add_option(pdu, COAP_OPTION_CONTENT_FORMAT, 1, &format)) { coap_delete_pdu(pdu); failure("format option failed"); coap_session_release(session); status=1; goto done; }
    }
    if(payload_length && !coap_add_data(pdu, payload_length, payload)) { coap_delete_pdu(pdu); failure("payload failed"); coap_session_release(session); status=1; goto done; }
    if(coap_send(session,pdu)==COAP_INVALID_MID){failure("send failed");status=1;} else {
      while(!complete) {if(elapsed_ns()>=(uint64_t)timeout*1000000ULL)break;if(coap_io_process(ctx,10)<0)break;}
      if(!complete){failure("request timed out");status=1;}
    }
    coap_session_release(session);
  }
done:coap_free_context(ctx);coap_cleanup();return status;
}
