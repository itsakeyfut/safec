// `t` is read out of `b`, which holds `x` alone, so storing it in `m` stores
// `x` and not every block stored anywhere: `x` was written before it is
// returned through `*m` (#920).
void *malloc(int n);
int **f(void) {
    int **x = malloc(8); if (!x) return 0;
    int *y = malloc(4); if (!y) return 0;
    *x = y;
    int ***m = malloc(8); if (!m) return 0;
    int ***b = malloc(8); if (!b) return 0;
    *b = x;
    int **t = *b;
    *m = t;
    return *m;
}
