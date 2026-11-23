union U {
    char c;
    long long l;
};

int sized[sizeof(union U) == 8 ? 1 : -1];

struct S {
    int a;
};

union S {
    int a;
};

union U *pointer(union U *u) {
    return u;
}

union U {
    int again;
};

struct V {
    int a;
};

union V;

union Flexible {
    int n;
    int rest[];
};

struct Tail {
    int n;
    int rest[];
};

union Holder {
    struct Tail tail;
    int other;
};

int incomplete(void) {
    union Later;
    return sizeof(union Later);
}
