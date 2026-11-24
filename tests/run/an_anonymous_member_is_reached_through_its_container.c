struct Value {
    int kind;
    union {
        long long whole;
        struct {
            int low;
            int high;
        };
    };
};

union Word {
    struct {
        unsigned char first;
        unsigned char second;
    };
    unsigned short both;
};

static int high_of(struct Value *v) {
    return v->high;
}

int main(void) {
    int wrong = 0;
    struct Value v;
    v.kind = 1;
    v.whole = 0;
    v.high = 2;
    v.low = 3;
    // `low` and `high` share the bytes of `whole`, low half first on a
    // little-endian target, and `kind` is beside them.
    wrong += v.whole == 0x200000003LL && v.kind == 1 ? 0 : 1;
    wrong += high_of(&v) == 2 ? 0 : 1;
    wrong += sizeof(struct Value) == 16 ? 0 : 1;

    union Word w;
    w.both = 0;
    w.second = 1;
    wrong += w.both == 0x100 && w.first == 0 ? 0 : 1;
    return wrong;
}
