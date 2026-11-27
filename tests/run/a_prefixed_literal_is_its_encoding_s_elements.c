unsigned short wide_static[] = u"xy";

int main(void) {
    // The elements, each of its prefix's type (C17 6.4.5 p6).
    const unsigned short *u16 = u"a\xffff";
    const unsigned int *u32 = U"z";
    const char *u8 = u8"é";
    if (u16[0] != 'a' || u16[1] != 0xffff || u16[2] != 0 || u32[0] != 'z') {
        return 1;
    }
    if ((unsigned char)u8[0] != 0xc3 || (unsigned char)u8[1] != 0xa9 || u8[2] != 0) {
        return 2;
    }
    // A character outside the Basic Multilingual Plane is two elements of
    // UTF-16, one of UTF-32, and four bytes of UTF-8; `L` is UTF-16 where
    // `wchar_t` has 16 bits.
    if (sizeof(u"😀") != 6 || sizeof(U"😀") != 8 || sizeof(u8"😀") != 5) {
        return 3;
    }
    if (sizeof(L"😀") / sizeof(L'a') != (sizeof(L'a') == 2 ? 3 : 2)) {
        return 4;
    }
    // A constant has its prefix's type, not `int` (6.4.4.4 p11).
    if (sizeof u'a' != 2 || sizeof U'a' != 4 || u'\xffff' != 0xffff || U'a' != 97) {
        return 5;
    }
    if (L'\xff' != 255 || L'b' != 98) {
        return 6;
    }
    // A run takes the prefix of its prefixed literal (6.4.5 p5).
    if (sizeof(u"a" "b") != 6 || sizeof("a" U"b") != 12) {
        return 7;
    }
    // An array of the element type from a wide literal (6.7.9 p15), in a
    // block, braced, longer than the literal, and in a compound literal.
    unsigned int w32[] = U"hi";
    unsigned short w16[6] = {u"ab"};
    const unsigned short *c16 = (unsigned short[]){u"cd"};
    if (sizeof w32 != 12 || w32[1] != 'i' || w32[2] != 0) {
        return 8;
    }
    if (w16[1] != 'b' || w16[2] != 0 || w16[5] != 0 || c16[1] != 'd') {
        return 9;
    }
    if (sizeof wide_static != 6 || wide_static[1] != 'y' || wide_static[2] != 0) {
        return 10;
    }
    return 0;
}
