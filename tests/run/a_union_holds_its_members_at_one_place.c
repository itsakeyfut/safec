union Word {
    unsigned char bytes[8];
    unsigned int low;
    unsigned long long whole;
};

struct Tagged {
    int kind;
    union {
        int number;
        long long wide;
    } value;
};

union Pair {
    int first;
    long long second;
};

static union Pair make(int value) {
    union Pair p;
    p.second = 0;
    p.first = value;
    return p;
}

static long long read(union Pair p) {
    return p.second;
}

int main(void) {
    int wrong = 0;
    union Word w;
    w.whole = 0;
    w.low = 0x01020304;
    // Every member starts at the same byte, so the `int` written is the low
    // half of the `long long` read, on a little-endian target.
    wrong += w.bytes[0] == 0x04 && w.whole == 0x01020304 ? 0 : 1;
    wrong += sizeof(union Word) == 8 && sizeof(union Pair) == 8 ? 0 : 1;

    struct Tagged t;
    t.kind = 1;
    t.value.wide = 0;
    t.value.number = 5;
    wrong += t.value.wide == 5 && t.kind == 1 ? 0 : 1;

    union Pair a = make(9);
    union Pair b;
    b = a;
    wrong += read(b) == 9 ? 0 : 1;
    return wrong;
}
