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
