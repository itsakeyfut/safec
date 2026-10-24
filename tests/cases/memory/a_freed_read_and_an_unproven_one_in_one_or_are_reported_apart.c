void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int **q;
    free(p);
    if (*p || (q = &p, *p)) { return 1; }
    return 0;
}
