void *malloc(int n);
void free(void *p);
int f(int *to, int *from, int count) {
    int n;
    n = (count + 3) / 4;
    switch (count % 4) {
    case 0:
        do {
            *to = *from;
    case 3:
            *to = *from;
    case 2:
            *to = *from;
    case 1:
            *to = *from;
            n = n - 1;
        } while (n > 0);
    }
    return 0;
}
