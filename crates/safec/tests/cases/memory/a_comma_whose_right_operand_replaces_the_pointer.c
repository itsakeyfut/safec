void *malloc(int n);
void free(void *p);
int f(int *q) {
    int *p = malloc(4);
    free(p);
    *p, p = q;
    return 0;
}
