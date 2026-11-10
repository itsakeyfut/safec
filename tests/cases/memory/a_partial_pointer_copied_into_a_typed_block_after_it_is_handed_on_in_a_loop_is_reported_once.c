void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void g(int **q);
void turns(int c) {
    int i = 1;
    int **m = malloc(16);
    if (!m) return;
    while (c) {
        g(m);
        memcpy(m, &i, 4);
        c = c - 1;
    }
}
