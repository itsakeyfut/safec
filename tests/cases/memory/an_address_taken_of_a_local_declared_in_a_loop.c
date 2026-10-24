void *malloc(int n);
void free(void *p);
int f(int n) {
    while (n) {
        int *p = malloc(4);
        free(p);
        int **pp = &p;
        n = n - 1;
    }
    return 0;
}
