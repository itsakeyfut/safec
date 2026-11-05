void *malloc(int n);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int x = 1;
    int *s = &x;
    int ***pp = malloc(8);
    if (pp == 0) {
        return 0;
    }
    memcpy(&s, *pp, 8);
    int *r = s;
    if (r == 0) {
        return 0;
    }
    *r = 1;
    return 0;
}
