#include <stdint.h>
#include <string.h>
#include "ucs2.h"

static const char HEX[] = "0123456789ABCDEF";

static int put_unit(char *out, size_t out_len, size_t *pos, uint16_t u)
{
    if (*pos + 4 >= out_len) {
        return -1;
    }
    for (int shift = 12; shift >= 0; shift -= 4) {
        out[(*pos)++] = HEX[(u >> shift) & 0xF];
    }
    return 0;
}

int utf8_to_ucs2_hex(const char *in, char *out, size_t out_len)
{
    const uint8_t *p = (const uint8_t *)in;
    size_t pos = 0;
    int units = 0;

    while (*p) {
        uint32_t cp;
        int extra;
        if (*p < 0x80) {
            cp = *p;
            extra = 0;
        } else if ((*p & 0xE0) == 0xC0) {
            cp = *p & 0x1F;
            extra = 1;
        } else if ((*p & 0xF0) == 0xE0) {
            cp = *p & 0x0F;
            extra = 2;
        } else if ((*p & 0xF8) == 0xF0) {
            cp = *p & 0x07;
            extra = 3;
        } else {
            return -1;
        }
        p++;
        for (int i = 0; i < extra; i++, p++) {
            if ((*p & 0xC0) != 0x80) {
                return -1;
            }
            cp = (cp << 6) | (*p & 0x3F);
        }

        if (cp >= 0x10000) {
            cp -= 0x10000;
            if (put_unit(out, out_len, &pos, 0xD800 | (cp >> 10)) ||
                put_unit(out, out_len, &pos, 0xDC00 | (cp & 0x3FF))) {
                return -1;
            }
            units += 2;
        } else {
            if (put_unit(out, out_len, &pos, (uint16_t)cp)) {
                return -1;
            }
            units++;
        }
    }
    if (pos >= out_len) {
        return -1;
    }
    out[pos] = '\0';
    return units;
}

static int hexval(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

static bool read_unit(const char *hex, uint32_t *u)
{
    *u = 0;
    for (int k = 0; k < 4; k++) {
        int v = hexval(hex[k]);
        if (v < 0) {
            return false;
        }
        *u = (*u << 4) | (uint32_t)v;
    }
    return true;
}

bool ucs2_hex_to_utf8(const char *hex, char *out, size_t out_len)
{
    size_t n = strlen(hex);
    if (n == 0 || n % 4) {
        return false;
    }

    size_t pos = 0;
    for (size_t i = 0; i < n; i += 4) {
        uint32_t u;
        if (!read_unit(hex + i, &u)) {
            return false;
        }
        if (u >= 0xD800 && u <= 0xDBFF && i + 4 < n) {
            uint32_t lo;
            if (read_unit(hex + i + 4, &lo) && lo >= 0xDC00 && lo <= 0xDFFF) {
                u = 0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00);
                i += 4;
            }
        }

        char buf[4];
        int len;
        if (u < 0x80) {
            buf[0] = (char)u;
            len = 1;
        } else if (u < 0x800) {
            buf[0] = (char)(0xC0 | (u >> 6));
            buf[1] = (char)(0x80 | (u & 0x3F));
            len = 2;
        } else if (u < 0x10000) {
            buf[0] = (char)(0xE0 | (u >> 12));
            buf[1] = (char)(0x80 | ((u >> 6) & 0x3F));
            buf[2] = (char)(0x80 | (u & 0x3F));
            len = 3;
        } else {
            buf[0] = (char)(0xF0 | (u >> 18));
            buf[1] = (char)(0x80 | ((u >> 12) & 0x3F));
            buf[2] = (char)(0x80 | ((u >> 6) & 0x3F));
            buf[3] = (char)(0x80 | (u & 0x3F));
            len = 4;
        }
        if (pos + len >= out_len) {
            return false;
        }
        memcpy(out + pos, buf, len);
        pos += len;
    }
    out[pos] = '\0';
    return true;
}

bool sms_is_gsm_safe(const char *text)
{
    static const char extra[] = " !\"#$%&'()*+,-./:;<=>?@_\r\n";
    for (const char *p = text; *p; p++) {
        char c = *p;
        if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
            continue;
        }
        if (!strchr(extra, c)) {
            return false;
        }
    }
    return true;
}
