void *malloc(int n);
void free(void *p);
void get(int **out, int *v);
int f(void) {
    int *u = malloc(4); if (!u) return 0; *u = 1;
    int k = 0;
    int *r = &k;
    get(&r, u);
    int *s = r;
    free(u);
    if (!s) return 0;
    return *s;
}
