struct Flags {
    unsigned ready : 1;
    unsigned count : 5;
    int delta : 4;
    char tag;
    unsigned long long wide : 40;
    unsigned : 3;
    unsigned last : 2;
};

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
    return wrong;
}
