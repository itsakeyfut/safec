void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    int *q = p + 1;
    int *r = q - 1;
    free(r);
    return 0;
}
