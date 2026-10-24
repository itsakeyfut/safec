void *malloc(int n);
void free(void *p);

int main(void) {
    int *p = malloc(8);
    int x;
    if (p == 0) {
        return 0;
    }
    *p = 0;
    x = (free(p + *p), 0);
    return x;
}
