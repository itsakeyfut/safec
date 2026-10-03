void *malloc(int n);
void free(void *p);
int f(int n) {
    while (n) {
        int *p;
        *p = 1;
        p = malloc(4);
        free(p);
        n = n - 1;
    }
    return 0;
}
