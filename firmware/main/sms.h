#pragma once

void sms_init(void);
void sms_on_urc(const char *line);

/* All of these queue work for the SMS task and return immediately. */
void sms_request_sync(void);
void sms_request_delete(int index);
void sms_request_send(const char *ref, const char *to, const char *body);
