char unsized[] = "ab";
char padded[5] = "xy";
char exact[2] = "ab";
const char braced[] = { "c" "d", };

static int locals(void) {
    char s[] = "ab";
    char t[2] = "ab";
    unsigned char u[] = { "\xff" };
    signed char v[3] = "\x80";
    const char w[] = "a\0b";
    if (sizeof s != 3 || s[0] != 'a' || s[1] != 'b' || s[2] != 0) {
        return 1;
    }
    if (sizeof t != 2 || t[0] != 'a' || t[1] != 'b') {
        return 2;
    }
    if (sizeof u != 2 || u[0] != 255 || u[1] != 0) {
        return 3;
    }
    if (v[0] != -128 || v[1] != 0 || v[2] != 0) {
        return 4;
    }
    if (sizeof w != 4 || w[1] != 0 || w[2] != 'b') {
        return 5;
    }
    return 0;
}

static int statics(void) {
    if (sizeof unsized != 3 || unsized[1] != 'b' || unsized[2] != 0) {
        return 6;
    }
    if (sizeof padded != 5 || padded[1] != 'y' || padded[2] != 0 || padded[4] != 0) {
        return 7;
    }
    if (sizeof exact != 2 || exact[1] != 'b') {
        return 8;
    }
    if (sizeof braced != 3 || braced[0] != 'c' || braced[1] != 'd') {
        return 9;
    }
    return 0;
}

int main(void) {
    int failed = locals();
    if (failed != 0) {
        return failed;
    }
    return statics();
}
