void *malloc(int *n);
void free(void *p);
int f(void) {
    int *q = malloc(0);
    malloc(q);
    free(q);
    free(q);
    return 0;
}
