void *memcpy(void *d, void *s, int n);
void *malloc(int n);
int f(void) {
    char (**pp)[16] = malloc(8);
    if (pp == 0) return 0;
    char c = 0;
    void *v = pp;
    char **cp = v;
    *cp = &c;
    if (*pp == 0) return 0;
    memcpy(pp, *pp, 16);
    return 0;
}
