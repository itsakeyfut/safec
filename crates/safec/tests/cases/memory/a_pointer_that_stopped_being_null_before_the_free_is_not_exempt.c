void free(void *p);
int f(int *q) {
    int *p = 0;
    free(q);
    p = q;
    free(p);
    return 0;
}
