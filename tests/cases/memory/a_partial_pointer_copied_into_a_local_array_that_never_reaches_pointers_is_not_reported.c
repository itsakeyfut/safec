void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
void out(int **pp) {
    int i = 1;
    char buf[8];
    if (!pp) return;
    int *m = malloc(8);
    if (!m) return;
    memcpy(buf, &i, 4);
    memcpy(m, buf, 8);
    *pp = m;
}
