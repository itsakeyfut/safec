void *malloc(int n);
void free(void *p);

int f(int n) {
    while (n) {
        int *p;
        if (n == 3) {
            free(p);
        }
        p = malloc(4);
        free(p);
        n = n - 1;
    }
    return 0;
}
