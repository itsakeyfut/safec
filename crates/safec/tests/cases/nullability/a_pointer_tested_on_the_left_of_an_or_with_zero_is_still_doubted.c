void *malloc(int n);
void free(void *p);
int g(void);

int f(void) {
    int *p = malloc(4);
    if (p != 0 || 0) {
        *p = 1;
    }
    free(p);
    return 0;
}
