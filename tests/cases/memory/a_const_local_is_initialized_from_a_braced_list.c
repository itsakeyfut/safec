struct Bits {
    int low : 3;
    int n;
};

struct In {
    int *p;
    int n;
};

struct Out {
    struct In in;
    int k;
};

int initialized(struct In in) {
    const int a[16] = {1};
    const struct {
        int *p;
        int n;
    } q = {0, 2};
    const struct Bits bits = {1};
    const struct Out out = {in, 1};
    return a[0] + q.n + bits.low + out.k;
}

int written(void) {
    const int a[2] = {1, 2};
    ((int *)a)[0] = 3;
    return a[0];
}
