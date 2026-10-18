void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    int *n = realloc(p, 8);
    p = c ? p : 0;
    if (!n) {
        if (p) free(p);
        return 0;
    }
    free(n);
    return 0;
}
