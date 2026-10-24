void *malloc(int n);
void free(void *p);
void *memset(void *s, int c, int n);
void forget(void);

int main(void) {
    int *p = malloc(4);
    memset(p, 0, 4);
    free(p);
    forget();
    free(p);
    return 0;
}
