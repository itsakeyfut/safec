void *malloc(int n);
void free(void *p);
void helper(int *p);

int f(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    helper(p);
    return c ? (free(p), *p) : *p;
}
