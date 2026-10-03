void *malloc(int n);
void free(void *p);
void *memset(void *p, int c, int n);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return (memset(a, 0, 4) != 0) + (free(a), 0);
}
