void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    const int *q = p;
    free(p);
    return *q;
}
