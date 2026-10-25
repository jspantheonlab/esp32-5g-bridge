#pragma once

#include <stdbool.h>
#include <stddef.h>

/* UTF-8 -> UCS2/UTF-16BE hex (as used by AT+CSCS="UCS2"). Returns UTF-16 unit count or -1. */
int utf8_to_ucs2_hex(const char *in, char *out, size_t out_len);

/* UCS2/UTF-16BE hex -> UTF-8. Returns false if the input is not valid hex UCS2. */
bool ucs2_hex_to_utf8(const char *hex, char *out, size_t out_len);

/* True if text only uses characters that map 1:1 into the GSM 7-bit default alphabet. */
bool sms_is_gsm_safe(const char *text);
