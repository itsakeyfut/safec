enum { Narrow = 3 };

struct Fine {
    int a : Narrow;
    unsigned : 2;
    int : 0;
    unsigned long long b : 40;
};

int sized[sizeof(struct Fine) == 16 ? 1 : -1];

struct Unfit {
    int wider : 33;
    int : -1;
    int named : 0;
    int *pointer : 2;
};

int n;

struct NotConstant {
    int x : n;
};

struct Trailing {
    int a;
    int : 0;
};

struct Fixed {
    const unsigned narrow : 3;
    int other;
};

void write_fixed(struct Fixed *f) {
    f->narrow = 1;
}

int address(struct Fine *f) {
    int *p = &f->a;
    return sizeof(f->a) + *p;
}
