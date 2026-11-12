// `b` names one site for every turn's block, and `*keep` holds the first
// turn's, which still holds `x` when `x` is freed. The second turn's copy of
// a null replaces only its own block, as its store would, so on that path
// the site's record has to keep `x` (#927).
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int *x = malloc(4);
    if (!x) return 0;
    int ***keep = malloc(8);
    if (!keep) return 0;
    *keep = 0;
    int *z = 0;
    for (int k = 0; k < 2; k++) {
        int **b = malloc(8);
        if (!b) return 0;
        if (k == 0) {
            *b = x;
            *keep = b;
        } else {
            memcpy(b, &z, 8);
            free(x);
            int **o = *keep;
            if (!o) return 0;
            int *r = *o;
            if (!r) return 0;
            return *r;
        }
    }
    return 0;
}
