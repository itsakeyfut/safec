// `v` is `&x` until a turn makes it the block `malloc` returned, and a
// later turn's `malloc` gives that block's name to a new allocation, so
// `v` loses it and is a value this check stopped following (`Why::Lost`).
// `*v = z` may then land in the old block rather than `x`, so `y = x` reads
// `x` while it may be indeterminate (C17 6.2.4 p6, 6.7.9 p10): with
// `k == 2`, `d == 0` and `c == 1`, it is (#946).
void *malloc(int n);

int *f(int k, int c, int d, int *z) {
    int *x;
    int **v = &x;
    int *y = 0;
    for (int i = 0; i < k; i++) {
        int **m = malloc(8);
        if (!m) return z;
        if (i == c) {
            *v = z;
            y = x;
            return z;
        }
        if (i == d) v = m;
    }
    return y;
}
