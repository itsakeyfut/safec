void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *q = p;
    free(q);
    free(p);
    return 0;
}
