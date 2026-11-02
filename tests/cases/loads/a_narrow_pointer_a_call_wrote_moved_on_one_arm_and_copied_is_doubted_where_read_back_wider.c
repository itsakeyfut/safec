void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void get(char **p);
int f(int k) {
    char *c;
    get(&c);
    char *t = 0;
    if (k) t = c;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &t, 8);
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
