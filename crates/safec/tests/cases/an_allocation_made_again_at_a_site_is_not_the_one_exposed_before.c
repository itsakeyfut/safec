void *malloc(int n);
void free(void *p);
void *memset(void *s, int c, int n);
void forget(void);

int main(void) {
    int i = 0;
    while (i < 2) {
        int *p = malloc(4);
        forget();
        memset(p, 0, 4);
        free(p);
        i = i + 1;
    }
    return 0;
}
