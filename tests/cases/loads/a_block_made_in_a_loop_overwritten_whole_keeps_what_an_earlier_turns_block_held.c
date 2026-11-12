// `b` names one site for every turn's block, and `*keep` holds the first
// turn's, which still holds `x` when `x` is freed. The second turn's `*b =
// 0;` replaces only its own block, and its arm reads on without going round
// again, so on that path the site's record has to keep `x` (#656, ADR-0094
// rule 1).
void *malloc(int n);
void free(void *p);
int f(void) {
    int *x = malloc(4);
    if (!x) return 0;
    int ***keep = malloc(8);
    if (!keep) return 0;
    *keep = 0;
    for (int k = 0; k < 2; k++) {
        int **b = malloc(8);
        if (!b) return 0;
        if (k == 0) {
            *b = x;
            *keep = b;
        } else {
            *b = 0;
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
