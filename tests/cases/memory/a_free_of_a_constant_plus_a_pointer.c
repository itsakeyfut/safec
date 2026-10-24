void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    free(1 + p);
    return 0;
}
