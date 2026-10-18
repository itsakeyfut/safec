void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    int *n = realloc(p, 8);
    if (c) p = 0;
    if (!n) {
        if (p) free(p);
        return 0;
    }
    if (p) *p = 1;
    free(n);
    return 0;
}
