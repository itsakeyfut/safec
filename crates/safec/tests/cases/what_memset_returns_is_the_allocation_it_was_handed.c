void *malloc(int n);
void free(void *p);
void *memset(void *s, int c, int n);

int main(void) {
    int *p = memset(malloc(4), 0, 4);
    free(p);
    return 0;
}
