void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(8);
    int *p;
    int *keep = a;
    while (c) {
        p = malloc(8);
        if (c > 1) { keep = p; }
        c = c - 1;
    }
    *keep = 1;
    return 0;
}
