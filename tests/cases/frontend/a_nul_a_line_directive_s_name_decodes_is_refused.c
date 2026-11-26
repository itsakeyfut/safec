#line 1 "a\0bcdefgh"
#define SIZE sizeof(__FILE__)

int main(void) {
    char buf[SIZE];
    buf[5] = 0;
    return buf[5];
}
