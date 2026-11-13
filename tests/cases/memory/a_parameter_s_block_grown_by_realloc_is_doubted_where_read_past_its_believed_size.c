// `p` is believed to point at one `int *` at least, so the caller's block may
// have been larger than eight bytes: `q[1]` may be what it held, or bytes
// nothing wrote (#918's review).
void *realloc(void *p, int n);
void free(void *p);
int f(int ** _Nonnull p) {
    int **q = realloc(p, 16);
    if (!q) return 0;
    int *r = q[1];
    free(q);
    return r == 0;
}
