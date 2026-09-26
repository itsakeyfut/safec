void *malloc(int n);
void free(void *p);
void *memset(void *s, int c, int n);
void forget(void);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    memset(p, 0, 4);
    forget();
    return *p;
}
