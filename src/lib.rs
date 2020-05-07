#![forbid(unsafe_code)]

pub mod raw;
pub mod framed;

pub use framed::LZ4FrameReader;




#[cfg(test)]
mod tests {
    use std::str;
    use crate::raw::compress2;
    use crate::raw::test::decompress;

    fn compress(input: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        if input.len() <= 0xFFFF {
            compress2(input, 0, &mut crate::raw::U16Table::default(), &mut buf).unwrap();
        } else {
            compress2(input, 0, &mut crate::raw::U32Table::default(), &mut buf).unwrap();
        }
        buf
    }

    /// Test that the compressed string decompresses to the original string.
    fn inverse(s: &str) {
        let compressed = compress(s.as_bytes());
        println!("Compressed '{}' into {:?}", s, compressed);
        let decompressed = decompress(&compressed).unwrap();
        println!("Decompressed it into {:?}", str::from_utf8(&decompressed).unwrap());
        assert_eq!(decompressed, s.as_bytes());
    }

    #[test]
    fn shakespear() {
        inverse("to live or not to live");
        inverse("Love is a wonderful terrible thing");
        inverse("There is nothing either good or bad, but thinking makes it so.");
        inverse("I burn, I pine, I perish.");
    }

    #[test]
    fn save_the_pandas() {
        inverse("To cute to die! Save the red panda!");
        inverse("You are 60% water. Save 60% of yourself!");
        inverse("Save water, it doesn't grow on trees.");
        inverse("The panda bear has an amazing black-and-white fur.");
        inverse("The average panda eats as much as 9 to 14 kg of bamboo shoots a day.");
        inverse("The Empress Dowager Bo was buried with a panda skull in her vault");
    }

    #[test]
    fn not_compressible() {
        inverse("as6yhol.;jrew5tyuikbfewedfyjltre22459ba");
        inverse("jhflkdjshaf9p8u89ybkvjsdbfkhvg4ut08yfrr");
    }

    #[test]
    fn short() {
        inverse("ahhd");
        inverse("ahd");
        inverse("x-29");
        inverse("x");
        inverse("k");
        inverse(".");
        inverse("ajsdh");
    }

    #[test]
    fn empty_string() {
        inverse("");
    }

    #[test]
    fn nulls() {
        inverse("\0\0\0\0\0\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn compression_works() {
        let s = "The Read trait allows for reading bytes from a source. Implementors of the Read trait are called 'readers'. Readers are defined by one required method, read().";

        inverse(s);

        assert!(compress(s.as_bytes()).len() < s.len());
    }

    #[test]
    fn big_compression() {
        let mut s = Vec::with_capacity(80_000000);

        for n in 0..80_000000 {
            s.push((n as u8).wrapping_mul(0xA).wrapping_add(33) ^ 0xA2);
        }

        assert_eq!(&decompress(&compress(&s)).unwrap(), &s);
    }
}
