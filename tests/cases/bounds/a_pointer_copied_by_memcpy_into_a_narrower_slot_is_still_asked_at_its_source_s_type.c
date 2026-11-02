void *memcpy(void *d, void *s, int n);
void use(void);
void f(char **out, char **cc) {
    if (!out) return;
    if (!cc) return;
    char *c = *cc;
    void *pv = &c;
    int **s = pv;
    memcpy(out, s, 8);
    use();
}
