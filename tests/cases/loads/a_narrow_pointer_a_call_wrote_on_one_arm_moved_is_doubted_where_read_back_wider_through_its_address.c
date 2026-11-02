void get(char **p);
int f(int k) {
    char *c = 0;
    if (k) get(&c);
    char *t = c;
    void *pv = &t;
    int **ib = pv;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
