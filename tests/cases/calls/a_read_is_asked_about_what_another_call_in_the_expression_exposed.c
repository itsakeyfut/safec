void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void **slot(void);
int release_all(void);

int main(void) {
    void **out_ = slot();
    int x;
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (x = a[0]) + (memcpy(out_, &a, 8) != 0) + release_all();
    return r;
}
