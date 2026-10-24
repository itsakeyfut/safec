void *malloc(int n);
void free(void *p);
void *memset(void *p, int c, int n);

int main(void) {
    int *a = malloc(4);
    int x;
    if (a == 0) {
        return 0;
    }
    x = (free(memset(a, 0, 4)), 0);
    return x;
}
