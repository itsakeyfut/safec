void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p;
    while (c) {
        p = malloc(8);
        free(p);
        c = c - 1;
    }
    return 0;
}
