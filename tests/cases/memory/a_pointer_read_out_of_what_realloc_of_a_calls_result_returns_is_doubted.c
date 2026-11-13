// What a call returns may be null, unless its callee promises it is
// not (#931).
void *realloc(void *p, int n);
void free(void *p);
int **get(void);
int g(void) {
    int **old = get();
    int **pp = realloc(old, 8);
    if (!pp) return 1;
    int *p = *pp;
    free(pp);
    return p != 0;
}
