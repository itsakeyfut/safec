static int dirty(void) {
    char junk[64];
    int sum = 0;
    for (int i = 0; i < 64; i++) {
        junk[i] = 85;
    }
    for (int i = 0; i < 64; i++) {
        sum += junk[i];
    }
    return sum;
}

static int filled(void) {
    char s[64] = "ab";
    int sum = 0;
    for (int i = 2; i < 64; i++) {
        sum += s[i];
    }
    return sum;
}

int main(void) {
    if (dirty() != 85 * 64) {
        return 1;
    }
    return filled();
}
