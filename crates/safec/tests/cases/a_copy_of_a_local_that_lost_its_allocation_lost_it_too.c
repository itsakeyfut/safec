void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p;
    int *keep = 0;
    while (c) {
        p = malloc(8);
        if (keep) {
            int *q = keep;
            *q = 1;
        }
        keep = p;
        free(p);
        c = c - 1;
    }
    return 0;
}
