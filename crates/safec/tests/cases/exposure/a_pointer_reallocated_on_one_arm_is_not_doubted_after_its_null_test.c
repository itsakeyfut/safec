void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    if (c) p = realloc(p, 8);
    if (!p) return 0;
    int r = *p;
    free(p);
    return r;
}
