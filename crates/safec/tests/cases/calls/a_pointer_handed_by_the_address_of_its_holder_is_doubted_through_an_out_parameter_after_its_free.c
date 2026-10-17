void *malloc(int n);
void free(void *p);
void get(int **out, int **w);
int f(int c) {
    int *u = malloc(4); if (!u) return 0; *u = 1;
    int *w = u;
    int *r = 0;
    get(&r, &w);
    int *s = r;
    free(u);
    if (!s) return 0;
    return *s;
}
