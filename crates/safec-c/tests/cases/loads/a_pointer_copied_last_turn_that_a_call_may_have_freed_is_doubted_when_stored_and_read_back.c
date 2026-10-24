void *malloc(int n);
void release_in(int **d);

int main(void) {
    int **c = malloc(8);
    if (c == 0) {
        return 0;
    }
    *c = 0;
    int *b = 0;
    int i = 0;
    while (i < 2) {
        int *a = malloc(4);
        if (a == 0) {
            return 0;
        }
        if (i == 1) {
            *c = b;
            int *old = *c;
            if (old != 0) {
                return *old;
            }
        }
        b = a;
        int **d = malloc(8);
        if (d == 0) {
            return 0;
        }
        *d = a;
        release_in(d);
        i = i + 1;
    }
    return 0;
}
