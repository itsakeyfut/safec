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

int address(struct Fine *f) {
    int *p = &f->a;
    return sizeof(f->a) + *p;
}
