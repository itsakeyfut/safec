void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(8);
    int *q;
    if (c) {
        q = p + 1;
    } else {
        q = p + 2;
    }
    free(q);
    return 0;
}
