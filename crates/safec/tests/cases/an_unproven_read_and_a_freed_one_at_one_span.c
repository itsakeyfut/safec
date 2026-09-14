void *malloc(int n);
void free(void *p);
void helper(void *p);
int f(void) {
    int *p = malloc(4);
    helper(p);
    if (*p || (free(p), *p)) { return 1; }
    return 0;
}
