void free(void *p);
int *lookup(void);

int f(int *p) {
    int *q = lookup();
    if (q == 0) {
        return 0;
    }
    free(p);
    return *q;
}
