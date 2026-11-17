void *malloc(int n);
void use(void);
void b(void ***pp) {
    if (!pp) return;
    void **q = malloc(8);
    if (!q) return;
    void *v = q;
    int *ip = v;
    *ip = 1;
    *q = v;
    *pp = q;
    use();
}
