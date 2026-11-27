static void *memset(void *into, int value, unsigned long long size) {
    char *bytes = into;
    for (unsigned long long i = 0; i < size; i++) {
        bytes[i] = 88;
    }
    return into;
}

static int memcpy = 3;

static int filled(void) {
    char s[4096] = "a";
    int sum = 0;
    for (int i = 1; i < 4096; i++) {
        sum += s[i];
    }
    return sum;
}

static int copied(void) {
    char s[300] = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    return s[199] == 'a' && s[200] == 0;
}

int main(void) {
    char used[4];
    memset(used, 0, sizeof used);
    if (used[3] != 88 || memcpy != 3) {
        return 1;
    }
    if (filled() != 0) {
        return 2;
    }
    return copied() ? 0 : 3;
}
