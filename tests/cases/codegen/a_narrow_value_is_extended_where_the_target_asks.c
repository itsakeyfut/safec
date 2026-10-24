char twice(char x);
void nothing(char *p);

char use_it(char a, int b, char *p) {
    nothing(p);
    return twice(a) + b;
}
