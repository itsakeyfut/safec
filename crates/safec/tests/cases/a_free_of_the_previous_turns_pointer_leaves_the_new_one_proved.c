void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = 0;
    int *keep = 0;
    while (c) {
        p = malloc(8);
        if (keep) { free(keep); }
        keep = p;
        c = c - 1;
    }
    free(p);
    return 0;
}
