void *malloc(int n);
void free(void *p);
void copy3(int **out, int ****src);
int f(int c) {
    int *u = malloc(4); if (!u) return 0; *u = 1;
    int *a = u;
    int **pa = &a;
    int ***ppa = &pa;
    int *r = 0;
    copy3(&r, &ppa);
    int *s = r;
    free(u);
    if (!s) return 0;
    return *s;
}
