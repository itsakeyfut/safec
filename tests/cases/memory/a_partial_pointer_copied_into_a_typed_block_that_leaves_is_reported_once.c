void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void typed(int ***pp) {
    int i = 1;
    if (!pp) return;
    int **q = malloc(16);
    if (!q) return;
    *pp = q;
    memcpy(q, &i, 4);
    use();
}
