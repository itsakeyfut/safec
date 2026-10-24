void *malloc(int n);
void *memset(void *p, int c, int n);
int release(int *p);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return (memset(a, 0, 4) != 0) + release(a);
}
