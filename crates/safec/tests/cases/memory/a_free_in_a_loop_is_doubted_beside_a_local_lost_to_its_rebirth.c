void *malloc(int n);
void free(void *p);
int f(int c, int n) {
    int *l = 0;
    for (int i = 0; i < n; i++) {
        int *p = malloc(4);
        if (!p) return 0;
        *p = 1;
        if (l) { *l = 2; }
        if (c) { free(p); p = 0; }
        if (p) { l = p; }
        if (!p) return 0;
        free(p);
    }
    return 0;
}
