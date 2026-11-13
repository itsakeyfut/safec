// A `_Nonnull` parameter is no null, so `realloc` copies the caller's
// block (#931).
void *realloc(void *p, int n);
void free(void *p);
int g(int ** _Nonnull old) {
    int **pp = realloc(old, 8);
    if (!pp) { free(old); return 1; }
    int *p = *pp;
    free(pp);
    return p != 0;
}
