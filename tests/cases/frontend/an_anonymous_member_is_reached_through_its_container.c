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

struct Inner {
    union {
        int a;
        int a;
    };
};

struct Tail {
    struct {
        int count;
    };
    int rest[];
};

struct Siblings {
    struct {
        int x;
    };
    struct {
        int x;
    };
};

struct Deep {
    struct {
        struct {
            int y;
        };
    };
    int y;
};
