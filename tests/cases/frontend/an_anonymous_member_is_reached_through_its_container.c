struct Tagged {
    int kind;
    union {
        int number;
        struct {
            int low;
            int high;
        };
    };
};

int read(struct Tagged *t) {
    return t->kind + t->number + t->high;
}

struct Twice {
    int a;
    union {
        int a;
        int b;
    };
};

int missing(struct Tagged t) {
    return t.nowhere;
}
