// The bytes `*n` holds came from `m` through `realloc`, so the report names
// the call and `m` (#752).
void *malloc(int n); void *realloc(void *p, int n);
int f(void) {
    int **m = malloc(8); if (!m) return 0;
    int **n = realloc(m, 16); if (!n) return 0;
    int *r = *n;
    return r == 0;
}
