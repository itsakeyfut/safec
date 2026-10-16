void *malloc(int n);
int make(int **out, int *in) {
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    *out = p;
    if (!in) return 0;
    return *in;
}
