void *malloc(int n);
int strlen(char *s);

int main(void) {
    char *p = malloc(1);
    if (p == 0) {
        return 1;
    }
    *p = 65;
    return strlen(p);
}
