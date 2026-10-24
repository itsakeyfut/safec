void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    int *q = 0;
    if (c) {
        q = realloc(p, 8);
    } else {
        q = p;
    }
    if (!q) return 0;
    free(q);
    return 0;
}
