void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(8);
    int *b = malloc(8);
    int *p;
    if (c) {
        p = a;
    } else {
        p = b;
    }
    free(p + 1);
    return 0;
}
