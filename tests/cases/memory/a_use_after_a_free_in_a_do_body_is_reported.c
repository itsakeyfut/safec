void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int n = 0;
    do {
        free(p);
        *p = 1;
        n = n + 1;
    } while (n < 3);
    return 0;
}
