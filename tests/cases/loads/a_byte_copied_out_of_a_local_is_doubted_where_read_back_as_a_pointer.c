void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(char *cc) {
    if (!cc) return 0;
    char b = *cc;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &b, 1);
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
