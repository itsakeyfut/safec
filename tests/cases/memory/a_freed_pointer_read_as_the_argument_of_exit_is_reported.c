void *malloc(int n);
void free(void *p);
void exit(int status);

int f(int n) {
    int *p = malloc(4);
    free(p);
    n > 0 || (exit(*p), 0);
    return 0;
}
