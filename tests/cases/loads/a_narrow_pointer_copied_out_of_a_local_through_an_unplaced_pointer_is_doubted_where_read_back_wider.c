void *memcpy(void *d, void *s, int n);
int f(char **w, char **cc, int **box) {
    if (!w) return 0;
    if (!cc) return 0;
    if (!box) return 0;
    char *c = *cc;
    memcpy(w, &c, 8);
    int *r = *box;
    if (!r) return 0;
    return *r;
}
