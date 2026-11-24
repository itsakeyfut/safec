struct Flags {
    unsigned ready : 1;
    unsigned count : 5;
    int delta : 4;
    char tag;
    unsigned long long wide : 40;
    unsigned : 3;
    unsigned last : 2;
};

struct Wide {
    unsigned long long low : 3;
    unsigned long flag : 1;
    int : 0;
};

struct Declarators {
    int pad;
    unsigned first : 2, second : 3;
    struct {
        int lo : 3;
        int hi : 5;
    };
    unsigned whole : 32;
    int full : 32;
};

static int first(struct Flags *f) {
    return f->ready;
}

/* A function after one that reaches a bit-field, whose `q` and `x` are
   numbered as that one's unit address and temporary were: an assignment
   here writes only `x`. */
static int unrelated(int y, unsigned char *q, int x) {
    if (!q) {
        return 0;
    }
    x = 200;
    return x == 200 && *q == 240 ? 0 : 1;
}

int main(void) {
    int wrong = 0;
    struct Flags f;
    f.tag = 'x';
    f.ready = 1;
    f.count = 31;
    f.delta = -3;
    f.wide = 0xFFFFFFFFFFull;
    f.last = 2;
    wrong += f.ready == 1 && f.count == 31 && f.delta == -3 ? 0 : 1;
    wrong += f.tag == 'x' && f.wide == 0xFFFFFFFFFFull && f.last == 2 ? 0 : 1;

    // A write keeps every other bit of its unit.
    f.count = 4;
    wrong += f.ready == 1 && f.delta == -3 ? 0 : 1;

    // The value of an assignment is what the bit-field then holds, cut to
    // its width, and a step past the top wraps.
    int stored = (f.count = 33);
    f.count += 31;
    f.delta++;
    wrong += stored == 1 && f.count == 0 && f.delta == -2 ? 0 : 1;

    // An `unsigned` bit-field narrower than `int` promotes to `int`, so
    // this subtraction is below zero (C17 6.3.1.1 p2).
    int promoted = f.ready - 2 < 0;
    wrong += promoted ? 0 : 1;

    // So does one of `unsigned long long` or `unsigned long`, as `clang`
    // promotes any bit-field narrower than `int`.
    struct Wide w;
    w.low = 0;
    w.flag = 0;
    int promoted_wide = w.low - 1 < 0 && w.flag - 1 < 0;
    wrong += promoted_wide ? 0 : 1;

    // Two declarators in one declaration, each with its width; bit-fields
    // in an anonymous struct, reached as the container's; and the widths
    // of `int` itself, where an `unsigned` one stays `unsigned`.
    struct Declarators d;
    d.pad = 0;
    d.first = 3;
    d.second = 5;
    d.lo = 3;
    d.hi = -2;
    d.whole = 0;
    d.full = -1;
    wrong += d.first == 3 && d.second == 5 && d.lo + d.hi == 1 ? 0 : 1;
    wrong += d.whole - 1 > 0 && d.full < 0 ? 0 : 1;

    unsigned char byte = 240;
    wrong += first(&f) == 1 ? 0 : 1;
    wrong += unrelated(0, &byte, 1);
    return wrong;
}
