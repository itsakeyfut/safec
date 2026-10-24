void *malloc(int n);
void free(void *p);
int f(int n) {
    int *p = malloc(4);
    int x;
    if (p == 0) return 0;
    x = (*p)++ + (n ? (free(p), 0) : 0);
    return x;
}
