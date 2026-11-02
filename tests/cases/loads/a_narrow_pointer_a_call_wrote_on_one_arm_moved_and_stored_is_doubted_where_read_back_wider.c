void *malloc(int n); void get(char **p);
int f(int k) {
    char *c = 0;
    if (k) get(&c);
    char *t = c;
    void *m = malloc(8);
    if (!m) return 0;
    char **cm = m; *cm = t;
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
