void *malloc(int n);
void free(void *p);
void get(int **pp);
void *realloc(void *p, int n);
int f(int c) {
    int *u; get(&u);
    int *q = malloc(4); if (!q) return 0;
    int *r;
    if (c) { r = u; } else { r = q; }
    int *s = realloc(r, 8);
    if (!s) { free(q); return 0; }
    free(s);
    return 0;
}
