/* Independent libcoap fixture. No Coaptic codecs or library code. */
#include <coap3/coap.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

static unsigned counter;
static uint8_t large_body[2000];
static const uint8_t small_body[] = "core-test-payload";
static int complete;
static coap_tick_t started;

static void failure(const char *message) {
  /* Only fixed diagnostics are passed, never unescaped external strings. */
  printf("{\"schema\":\"coaptic-peer/1\",\"event\":\"error\",\"message\":\"%s\"}\n", message);
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
static coap_response_t response_handler(coap_session_t *session,
    const coap_pdu_t *sent,const coap_pdu_t *received,const coap_mid_t mid) {
  (void)session;(void)sent;(void)mid;
  const uint8_t *data=NULL;size_t len=0,offset=0,total=0;
  coap_tick_t now;coap_ticks(&now);
  coap_get_data_large(received,&len,&data,&offset,&total);
  if (offset || total>len) return COAP_RESPONSE_FAIL;
  printf("{\"schema\":\"coaptic-peer/1\",\"event\":\"response\",\"code\":%u,\"payload_hex\":\"",(unsigned)coap_pdu_get_code(received));
  for(size_t i=0;i<len;i++)printf("%02x",data[i]);
  printf("\",\"elapsed_us\":%llu}\n",(unsigned long long)((now-started)*1000000/COAP_TICKS_PER_SECOND));
  complete=1;return COAP_RESPONSE_OK;
}
int main(int argc,char **argv) {
  setvbuf(stdout,NULL,_IONBF,0);
  if(argc!=8){failure("invalid arguments");return 2;}
  const int server=strcmp(argv[1],"server")==0,dtls=strcmp(argv[2],"dtls")==0;
  if ((!server && strcmp(argv[1],"client")) || (!dtls && strcmp(argv[2],"udp"))) {failure("invalid mode");return 2;}
  char *end=NULL;long port=strtol(argv[3],&end,10);
  if(*end || port<1 || port>65535){failure("invalid port");return 2;}
  long timeout=strtol(argv[7],&end,10);
  if(*end || timeout<100 || timeout>30000){failure("invalid timeout");return 2;}
  if(strcmp(argv[5],"test") && strcmp(argv[5],"large") && strcmp(argv[5],"counter") && strcmp(argv[5],"missing")){failure("unsupported path");return 2;}
  if(strcmp(argv[6],"GET") && strcmp(argv[6],"POST")){failure("unsupported method");return 2;}
  coap_startup();coap_set_log_level(COAP_LOG_EMERG);
  if(dtls && !coap_dtls_is_supported()){failure("DTLS unavailable in libcoap build");coap_cleanup();return 2;}
  coap_ticks(&started);
  for(size_t i=0;i<sizeof(large_body);i++)large_body[i]=(uint8_t)(i%251);
  coap_context_t *ctx=coap_new_context(NULL);
  if(!ctx){failure("context failed");coap_cleanup();return 1;}
  coap_context_set_block_mode(ctx,COAP_BLOCK_USE_LIBCOAP|COAP_BLOCK_SINGLE_BODY);
  coap_address_t addr;coap_address_init(&addr);
  addr.addr.sin.sin_family=AF_INET;addr.addr.sin.sin_addr.s_addr=htonl(INADDR_LOOPBACK);addr.addr.sin.sin_port=htons((uint16_t)port);addr.size=sizeof(addr.addr.sin);
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
    printf("{\"schema\":\"coaptic-peer/1\",\"event\":\"ready\",\"peer\":\"libcoap\",\"stack\":\"libcoap %s\",\"port\":%ld,\"transport\":\"%s\"}\n",LIBCOAP_PACKAGE_VERSION,port,dtls?"dtls":"udp");
    while(coap_io_process(ctx,100)>=0) {}
    status=1;
  } else {
    coap_session_t *session=dtls?coap_new_client_session_psk(ctx,NULL,&addr,proto,"password",(const uint8_t *)argv[4],(unsigned)strlen(argv[4])):coap_new_client_session(ctx,NULL,&addr,proto);
    if(!session){failure("session failed");status=1;goto done;}
    coap_register_response_handler(ctx,response_handler);
    coap_pdu_t *pdu=coap_new_pdu(COAP_MESSAGE_CON,strcmp(argv[6],"POST")==0?COAP_REQUEST_CODE_POST:COAP_REQUEST_CODE_GET,session);
    if(!pdu){failure("PDU allocation failed");coap_session_release(session);status=1;goto done;}
    uint8_t token[8];size_t token_len=sizeof(token);coap_session_new_token(session,&token_len,token);
    if(!coap_add_token(pdu,token_len,token)||!coap_add_option(pdu,COAP_OPTION_URI_PATH,strlen(argv[5]),(const uint8_t *)argv[5])){coap_delete_pdu(pdu);failure("PDU construction failed");coap_session_release(session);status=1;goto done;}
    if(coap_send(session,pdu)==COAP_INVALID_MID){failure("send failed");status=1;} else {
      while(!complete) {coap_tick_t now;coap_ticks(&now);if((now-started)*1000/COAP_TICKS_PER_SECOND>=(unsigned long)timeout)break;if(coap_io_process(ctx,10)<0)break;}
      if(!complete){failure("request timed out");status=1;}
    }
    coap_session_release(session);
  }
done:coap_free_context(ctx);coap_cleanup();return status;
}
